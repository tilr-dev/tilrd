//! The Unix socket for local clients: `tilr` on this machine reaches the
//! server here without the access token (see [`tilr_proto::local`] for where
//! it is and what is next to it).
//!
//! One server serves a socket: it holds `<socket>.lock` (`flock`) for as long
//! as it runs, a reload included (the descriptor is handed over with the
//! socket), with its pid written in it. A socket whose lock is free and that
//! refuses connections was left by a server that died (SIGKILL) and is
//! replaced; anything else at the path (a socket some other program listens
//! on, a file that is not a socket, a socket of another user) is an error,
//! and nothing is removed.
//!
//! Who may connect: the directory is the user's only (0700; the runtime
//! directory is made so), the socket is 0600, and each connection's peer
//! credentials must name this user, which also holds where the permissions
//! were loosened by hand.

use std::fs::{self, File};
use std::io::{ErrorKind, Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use tilr_proto::local;
use tokio::net::{UnixListener, UnixStream};

use crate::{clipboard, pty};

/// How often taking the lock is tried again when its file was replaced
/// between opening and locking it (another server removing it on exit).
const LOCK_TRIES: usize = 5;

/// The socket a server serves, and the lock that makes it the only one.
pub struct Local {
    pub listener: Arc<UnixListener>,
    lock: File,
    path: PathBuf,
}

/// Why a server could not take its socket.
pub enum ClaimError {
    /// Another server serves it.
    Busy {
        pid: Option<u32>,
    },
    Other(anyhow::Error),
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { pid: Some(pid) } => write!(f, "another tilrd (pid {pid}) serves it"),
            Self::Busy { pid: None } => write!(f, "another tilrd serves it"),
            Self::Other(err) => write!(f, "{err:#}"),
        }
    }
}

impl From<anyhow::Error> for ClaimError {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err)
    }
}

/// Take the socket at `path` and listen on it. `lock`: the descriptor of its
/// lock file, already locked by the `tilr` that started this server.
pub fn claim(path: &Path, lock: Option<RawFd>) -> Result<Local, ClaimError> {
    local::check_socket_path(path).map_err(|err| anyhow!(err))?;
    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .context("the socket path has no directory")?;
    if local::is_default(path) {
        clipboard::private_dir(dir)?;
    } else {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("create {}", dir.display()))?;
    }
    let lock_path = local::lock_path(path);
    let mut lock = match lock {
        Some(fd) => adopt_lock(fd, &lock_path)?,
        None => take_lock(&lock_path)?,
    };
    // Who holds it, for the error of the next one to try.
    let _ = lock
        .set_len(0)
        .and_then(|()| lock.rewind())
        .and_then(|()| writeln!(lock, "{}", std::process::id()));

    clear_path(path)?;
    let listener = std::os::unix::net::UnixListener::bind(path)
        .with_context(|| format!("listen on {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restrict {}", path.display()))?;
    listener
        .set_nonblocking(true)
        .context("socket non-blocking")?;
    let listener = UnixListener::from_std(listener).context("socket")?;
    Ok(Local {
        listener: Arc::new(listener),
        lock,
        path: path.to_owned(),
    })
}

/// Take over the socket and its lock from the server this one replaced.
pub fn inherit(listener: RawFd, lock: RawFd, path: PathBuf) -> Result<Local> {
    pty::set_cloexec(listener, true)?;
    pty::set_cloexec(lock, true)?;
    // SAFETY: the supervisor that exec'd this process left both descriptors
    // open for it and nothing else here owns them.
    let (listener, lock) = unsafe {
        (
            std::os::unix::net::UnixListener::from_raw_fd(listener),
            File::from_raw_fd(lock),
        )
    };
    listener.set_nonblocking(true)?;
    Ok(Local {
        listener: Arc::new(UnixListener::from_std(listener)?),
        lock,
        path,
    })
}

impl Local {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The descriptors a reload hands over: the listening socket and the
    /// lock.
    pub fn fds(&self) -> (RawFd, RawFd) {
        (self.listener.as_raw_fd(), self.lock.as_raw_fd())
    }

    /// The server stops for good: the socket and the lock file go, then the
    /// lock is let go of.
    pub fn remove(self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|meta| meta.file_type().is_socket()) {
            let _ = fs::remove_file(&self.path);
        }
        let lock_path = local::lock_path(&self.path);
        if same_file(&self.lock, &lock_path) {
            let _ = fs::remove_file(&lock_path);
        }
        drop(self.lock);
    }
}

/// Open the lock file and lock it, or say who holds it.
fn take_lock(path: &Path) -> Result<File, ClaimError> {
    for _ in 0..LOCK_TRIES {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .with_context(|| {
                format!("open {} (a symbolic link is not followed)", path.display())
            })?;
        own_file(&file, path)?;
        if !flock(&file).with_context(|| format!("lock {}", path.display()))? {
            let mut pid = String::new();
            let _ = file.read_to_string(&mut pid);
            return Err(ClaimError::Busy {
                pid: pid.trim().parse().ok(),
            });
        }
        // Locked, but the file may have been removed (a server stopping)
        // between opening and locking it: then it locks nothing.
        if same_file(&file, path) {
            return Ok(file);
        }
    }
    Err(anyhow!("{} keeps being replaced", path.display()).into())
}

/// The lock a `tilr` took and passed on: it must be the lock file's, and
/// locked.
fn adopt_lock(fd: RawFd, path: &Path) -> Result<File, ClaimError> {
    pty::set_cloexec(fd, true).context("the lock descriptor passed")?;
    // SAFETY: the process that started this one left the descriptor open for
    // it (--lock-fd) and nothing else here owns it.
    let file = unsafe { File::from_raw_fd(fd) };
    if !same_file(&file, path) {
        return Err(anyhow!(
            "descriptor {fd} passed as the lock is not {}",
            path.display()
        )
        .into());
    }
    // Locking again what this open file already holds succeeds at once.
    if !flock(&file).with_context(|| format!("lock {}", path.display()))? {
        return Err(ClaimError::Busy { pid: None });
    }
    Ok(file)
}

/// `flock(LOCK_EX | LOCK_NB)`: `false` if another holds it.
fn flock(file: &File) -> std::io::Result<bool> {
    // SAFETY: flock has no memory-safety preconditions; the descriptor is
    // open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    if err.kind() == ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(err)
    }
}

/// What was opened as the lock must be a regular file of this user's.
fn own_file(file: &File, path: &Path) -> Result<()> {
    let meta = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?;
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if !meta.is_file() {
        bail!(
            "{} is not a regular file (nothing was changed)",
            path.display()
        );
    }
    if meta.uid() != uid {
        bail!(
            "{} belongs to uid {}, not to this user (nothing was changed)",
            path.display(),
            meta.uid()
        );
    }
    Ok(())
}

/// Whether `file` is what is at `path` (a link there is not followed).
fn same_file(file: &File, path: &Path) -> bool {
    match (file.metadata(), fs::symlink_metadata(path)) {
        (Ok(open), Ok(named)) => open.dev() == named.dev() && open.ino() == named.ino(),
        _ => false,
    }
}

/// Make room for the socket at `path`, under the lock: only a socket of this
/// user that nobody listens on (a server killed outright left it) is
/// removed.
fn clear_path(path: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("stat {}", path.display())),
    };
    if !meta.file_type().is_socket() {
        bail!(
            "{} is in the way: it is not a socket (nothing was removed)",
            path.display()
        );
    }
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if meta.uid() != uid {
        bail!(
            "{} belongs to uid {}, not to this user (nothing was removed)",
            path.display(),
            meta.uid()
        );
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => bail!(
            "something that is not a tilrd listens on {} (nothing was removed): \
             stop it, or set {} to another path",
            path.display(),
            local::SOCKET_ENV
        ),
        Err(err) if err.kind() == ErrorKind::ConnectionRefused => {
            tracing::info!(path = %path.display(), "removing the socket of a server that is gone");
            fs::remove_file(path).with_context(|| format!("remove {}", path.display()))
        }
        Err(err) => Err(err).with_context(|| format!("connect to {}", path.display())),
    }
}

/// Who is on the other end of a local connection: its pid if the system
/// says, or why it may not come in (another user).
pub fn peer(stream: &UnixStream) -> Result<Option<i32>> {
    let cred = stream.peer_cred().context("peer credentials")?;
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if cred.uid() != uid {
        bail!(
            "uid {} (pid {:?}) is not this server's user {uid}",
            cred.uid(),
            cred.pid()
        );
    }
    Ok(cred.pid())
}

/// Whether [`peer`] failed only because the other end had already closed:
/// what a probe leaves (a connect to see whether a server is there, as a
/// starting server makes and as tmux's client does), and nothing to warn
/// about. macOS cannot tell who it was then (`ENOTCONN`); Linux can, so
/// there the check passes and the connection simply ends.
pub fn peer_gone(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|err| err.raw_os_error() == Some(libc::ENOTCONN))
}
