//! The local tilrd, tmux-style: `tilr` starts it when it is not running and
//! stops it with `kill-server`.
//!
//! Starting: the socket's lock file (see [`tilr_proto::local`]) is what one
//! running server holds. A `tilr` that finds no server takes the lock
//! itself and hands it to the tilrd it starts (`--lock-fd`), so of two
//! `tilr` started at once on an empty machine, one starts a server and the
//! other waits for its socket. The server runs detached: in a session of its
//! own (no controlling terminal), a grandchild of `tilr` (not its child),
//! with stdin from /dev/null and its log in `tilrd.log` next to the socket.

use std::fs::{self, File};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tilr_proto::local;

/// Names the server binary when it is neither next to `tilr` nor on PATH.
pub const TILRD_ENV: &str = "TILRD";
/// How long a started server may take to listen.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `kill-server` waits for the server and its panes to go.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(25);

/// What is at the socket's path.
enum Probe {
    /// Something accepts connections.
    Up,
    /// Nothing: no file, or a socket nobody accepts on.
    Down,
}

fn probe(socket: &Path) -> Result<Probe> {
    check_dir(socket)?;
    match fs::symlink_metadata(socket) {
        Ok(meta) if !meta.file_type().is_socket() => bail!(
            "{} is in the way: it is not a socket (nothing was removed); \
             move it, or set {} to another path",
            socket.display(),
            local::SOCKET_ENV
        ),
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Probe::Down),
        Err(err) => return Err(err).with_context(|| format!("stat {}", socket.display())),
    }
    match std::os::unix::net::UnixStream::connect(socket) {
        Ok(_) => Ok(Probe::Up),
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::ConnectionRefused | ErrorKind::NotFound
            ) =>
        {
            Ok(Probe::Down)
        }
        Err(err) => Err(err).with_context(|| format!("connect to {}", socket.display())),
    }
}

/// Whether a tilrd (or something) accepts connections on `socket`.
pub fn running(socket: &Path) -> bool {
    matches!(probe(socket), Ok(Probe::Up))
}

/// Make sure a tilrd serves `socket`: start one if none does. Returns once
/// the socket accepts connections.
pub fn ensure_running(socket: &Path) -> Result<()> {
    local::check_socket_path(socket).map_err(|err| anyhow!(err))?;
    if let Probe::Up = probe(socket)? {
        return Ok(());
    }
    prepare_dir(socket)?;
    let lock_path = local::lock_path(socket);
    let deadline = Instant::now() + START_TIMEOUT;
    let mut started: Option<PathBuf> = None;
    loop {
        if let Probe::Up = probe(socket)? {
            return Ok(());
        }
        // Locked: nobody serves the socket, or the server just started died.
        // Not: a server is there, or another tilr is starting one.
        if let Some(lock) = try_lock(&lock_path)? {
            if let Some(log) = &started {
                bail!(
                    "tilrd stopped before it listened on {}:\n{}",
                    socket.display(),
                    log_tail(log)
                );
            }
            started = Some(start(socket, &lock)?);
        }
        if Instant::now() >= deadline {
            let log = local::log_path(socket);
            bail!(
                "no tilrd listens on {} after {}s; its log ({}):\n{}",
                socket.display(),
                START_TIMEOUT.as_secs(),
                log.display(),
                log_tail(&log)
            );
        }
        std::thread::sleep(POLL);
    }
}

/// The runtime directory, when the socket is the default one there, must be
/// the user's alone (see [`local::check_runtime_dir`]) before anything at the
/// socket is reached or started.
fn check_dir(socket: &Path) -> Result<()> {
    if local::is_default(socket)
        && let Some(dir) = socket.parent()
    {
        local::check_runtime_dir(dir).map_err(|err| anyhow!(err))?;
    }
    Ok(())
}

/// The socket's directory: the runtime directory is made private to the
/// user, as the server makes it, and checked again once it is there.
fn prepare_dir(socket: &Path) -> Result<()> {
    let dir = socket
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .context("the socket path has no directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("create {}", dir.display()))?;
    check_dir(socket)
}

/// The lock file, locked; `None` if another holds it. A link at its path is
/// not followed, and what is there must be a file of this user's; it is not
/// truncated (the server writes its pid once it holds the lock).
fn try_lock(path: &Path) -> Result<Option<File>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("open {} (a symbolic link is not followed)", path.display()))?;
    own_file(&file, path)?;
    // SAFETY: flock has no memory-safety preconditions; the file is open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == ErrorKind::WouldBlock {
            return Ok(None);
        }
        return Err(err).with_context(|| format!("lock {}", path.display()));
    }
    // A server that stopped removes the file: the lock taken on it, between
    // opening and locking, locks nothing. Taken again next time round.
    let same = match (file.metadata(), fs::symlink_metadata(path)) {
        (Ok(open), Ok(named)) => open.dev() == named.dev() && open.ino() == named.ino(),
        _ => false,
    };
    Ok(same.then_some(file))
}

/// What was opened at `path` must be a regular file of this user's.
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
            "{} belongs to another user (uid {}), not to this one (uid {uid})",
            path.display(),
            meta.uid()
        );
    }
    Ok(())
}

/// Start tilrd for `socket`, handing it `lock`. Returns its log's path.
fn start(socket: &Path, lock: &File) -> Result<PathBuf> {
    let exe = find_tilrd()?;
    let log_path = local::log_path(socket);
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&log_path)
        .with_context(|| {
            format!(
                "open {} (a symbolic link is not followed)",
                log_path.display()
            )
        })?;
    own_file(&log, &log_path)?;
    let fd = lock.as_raw_fd();
    let mut command = Command::new(&exe);
    command
        .arg("--socket")
        .arg(socket)
        .arg("--lock-fd")
        .arg(fd.to_string())
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        // The server outlives this directory being anything in particular.
        .current_dir("/")
        // What says which pane `tilr` runs in is not the server's to keep.
        .env_remove("TILR_PANE")
        .env_remove("TILR_SOCK")
        .env_remove("TILR_SERVER")
        // Nor the service manager of a server it may run in.
        .env_remove("NOTIFY_SOCKET");
    // SAFETY: only async-signal-safe calls between fork and exec: setsid,
    // fcntl, fork, _exit.
    unsafe {
        command.pre_exec(move || {
            // The lock goes to the server.
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Fork once more and let the middle process go: the server is
            // then nobody's child here.
            match libc::fork() {
                -1 => return Err(std::io::Error::last_os_error()),
                0 => {}
                _ => libc::_exit(0),
            }
            // A session of its own, which it leads, with no terminal.
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut middle = command
        .spawn()
        .with_context(|| format!("start {}", exe.display()))?;
    let _ = middle.wait();
    Ok(log_path)
}

/// The tilrd to start: next to this executable, else on PATH, else
/// `$TILRD`.
pub fn find_tilrd() -> Result<PathBuf> {
    let own = std::env::current_exe().ok();
    let beside = own
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join("tilrd"));
    let on_path = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("tilrd"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let named = std::env::var_os(TILRD_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    beside
        .into_iter()
        .chain(on_path)
        .chain(named)
        .find(|path| is_executable(path))
        .with_context(|| {
            format!(
                "no tilrd to start: none next to {}, none on PATH, and ${TILRD_ENV} is not set",
                own.as_deref().unwrap_or(Path::new("tilr")).display()
            )
        })
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The end of a log, to show why a server did not start.
fn log_tail(path: &Path) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::from("(no log)");
    };
    let len = file.metadata().map_or(0, |meta| meta.len());
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(4000)));
    let mut text = String::new();
    let _ = file.read_to_string(&mut text);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(15)..].join("\n")
}

/// Whether nobody serves `socket` any more: its lock file is gone or free
/// (one that cannot be opened, another user's, is taken as held).
pub fn stopped(socket: &Path) -> bool {
    let lock = local::lock_path(socket);
    let file = fs::OpenOptions::new()
        .read(true)
        // Not a link; nor waiting for a writer, were it a FIFO.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&lock);
    match file {
        Ok(file) => {
            // SAFETY: flock has no memory-safety preconditions.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) == 0 }
        }
        Err(err) => err.kind() == ErrorKind::NotFound,
    }
}

/// Whether a process is there (and not a zombie waiting to be reaped).
pub fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_none_or(|state| state != "Z" && state != "X"),
        Err(_) => true,
    }
}

/// Stop the server: SIGTERM, then wait for it to let go of its socket and for
/// the pane processes (`panes`) to end.
pub fn stop(socket: &Path, server: i32, panes: &[i32]) -> Result<()> {
    // SAFETY: kill has no memory-safety preconditions.
    if unsafe { libc::kill(server, libc::SIGTERM) } != 0 {
        return Err(std::io::Error::last_os_error())
            .context(format!("signal tilrd (pid {server})"));
    }
    let deadline = Instant::now() + STOP_TIMEOUT;
    while !(stopped(socket) && !alive(server) && panes.iter().all(|&pid| !alive(pid))) {
        if Instant::now() >= deadline {
            bail!(
                "tilrd (pid {server}) did not stop within {}s",
                STOP_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}
