//! Where the server of this user on this machine listens for local clients:
//! a Unix socket, found by the server and its clients (`tilr`) alike.
//!
//! The socket is `$TILR_SOCKET` if set, else `tilrd.sock` in the runtime
//! directory: `$TILR_RUNTIME_DIR`, else `$XDG_RUNTIME_DIR/tilr`, else
//! `tilr-<uid>` in the temporary directory (`$TMPDIR` on macOS, `/tmp` on
//! Linux). Next to it: `<socket>.lock`, held (`flock`) by the one server
//! that serves the socket, with its pid in it; `tilrd.log`, the log of a
//! server `tilr` started.
//!
//! The protocol on the socket is the one on the TCP port (WebSocket, then
//! `Hello`), without the access token: the directory is the user's only, the
//! socket is 0600, and the server checks that a client's uid is its own.
//! `tilr` checks the other way round before it sends anything: the runtime
//! directory ([`check_runtime_dir`]) and the server's uid.

use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Overrides the socket's path.
pub const SOCKET_ENV: &str = "TILR_SOCKET";
/// Overrides where the server keeps its per-user files.
pub const RUNTIME_DIR_ENV: &str = "TILR_RUNTIME_DIR";
/// The socket's name in the runtime directory.
pub const SOCKET_NAME: &str = "tilrd.sock";
/// The URL a WebSocket handshake on the socket asks for: any will do, the
/// server serves the main protocol on every path.
pub const URL: &str = "ws://localhost/";

/// Directory for the server's per-user files, private to the user.
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(RUNTIME_DIR_ENV).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        return Path::new(&dir).join("tilr");
    }
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    temp_dir().join(format!("tilr-{uid}"))
}

/// `$TMPDIR` on macOS (a directory of the user's own), `/tmp` elsewhere.
fn temp_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        std::env::temp_dir()
    } else {
        PathBuf::from("/tmp")
    }
}

/// The socket: `$TILR_SOCKET`, else [`SOCKET_NAME`] in [`runtime_dir`].
pub fn socket_path() -> PathBuf {
    match std::env::var_os(SOCKET_ENV).filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => default_socket_path(),
    }
}

/// The socket when `$TILR_SOCKET` does not say.
pub fn default_socket_path() -> PathBuf {
    runtime_dir().join(SOCKET_NAME)
}

/// Whether `path` is where the socket goes when nothing says otherwise: its
/// directory is then the runtime directory, which the server keeps private.
pub fn is_default(path: &Path) -> bool {
    path == default_socket_path()
}

/// The lock file of the server that serves `socket`.
pub fn lock_path(socket: &Path) -> PathBuf {
    with_suffix(socket, ".lock")
}

/// The log of a server started in the background for `socket`:
/// `tilrd.log` next to `tilrd.sock`.
pub fn log_path(socket: &Path) -> PathBuf {
    let stem = socket.file_stem().unwrap_or(socket.as_os_str());
    let mut name = OsString::from(stem);
    name.push(".log");
    socket.with_file_name(name)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut path = path.as_os_str().to_owned();
    path.push(suffix);
    PathBuf::from(path)
}

/// Longest path a Unix socket can be bound or reached at: `sun_path` less its
/// terminating zero (103 bytes on macOS, 107 on Linux).
pub fn max_socket_path() -> usize {
    // SAFETY: sockaddr_un is plain data; all zeroes is a valid value.
    let addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_path.len() - 1
}

/// Why `path` cannot be a socket, if it cannot.
pub fn check_socket_path(path: &Path) -> Result<(), String> {
    let len = path.as_os_str().len();
    let max = max_socket_path();
    if len > max {
        return Err(format!(
            "the socket path {} is {len} bytes, longer than a Unix socket's {max}: \
             set {SOCKET_ENV} (or TILR_RUNTIME_DIR, or XDG_RUNTIME_DIR) to a shorter one",
            path.display()
        ));
    }
    Ok(())
}

/// Why the runtime directory `dir` cannot be trusted with the socket, if it
/// cannot (tmux's check of its own): it must be a directory itself, not a
/// link to one, owned by this user, with nothing for group and others. One
/// that is not there yet is fine: it is made private.
pub fn check_runtime_dir(dir: &Path) -> Result<(), String> {
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("the runtime directory {}: {err}", dir.display())),
    };
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    check_dir_meta(dir, &meta, uid)
}

fn check_dir_meta(dir: &Path, meta: &std::fs::Metadata, uid: u32) -> Result<(), String> {
    let dir = dir.display();
    let way_out = format!("remove it, or set {RUNTIME_DIR_ENV} to a directory of your own");
    if meta.file_type().is_symlink() {
        return Err(format!(
            "the runtime directory {dir} is a symbolic link, not a directory: {way_out}"
        ));
    }
    if !meta.is_dir() {
        return Err(format!(
            "the runtime directory {dir} is not a directory: {way_out}"
        ));
    }
    if meta.uid() != uid {
        return Err(format!(
            "the runtime directory {dir} belongs to another user (uid {}), not to this one \
             (uid {uid}): {way_out}",
            meta.uid()
        ));
    }
    let mode = meta.mode() & 0o7777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "the runtime directory {dir} is open to other users (mode {mode:04o}): \
             `chmod 700 {dir}`, or {way_out}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_go_next_to_the_socket() {
        let socket = Path::new("/run/user/1000/tilr/tilrd.sock");
        assert_eq!(
            lock_path(socket),
            Path::new("/run/user/1000/tilr/tilrd.sock.lock")
        );
        assert_eq!(log_path(socket), Path::new("/run/user/1000/tilr/tilrd.log"));
        assert_eq!(log_path(Path::new("/tmp/s")), Path::new("/tmp/s.log"));
    }

    #[test]
    fn a_path_longer_than_sun_path_is_refused_with_the_way_out() {
        let max = max_socket_path();
        assert!(max == 103 || max == 107, "{max}");
        let fits = PathBuf::from(format!("/{}", "a".repeat(max - 1)));
        assert!(check_socket_path(&fits).is_ok());
        let long = PathBuf::from(format!("/{}", "a".repeat(max)));
        let err = check_socket_path(&long).unwrap_err();
        assert!(err.contains(SOCKET_ENV), "{err}");
        assert!(err.contains(&format!("{} bytes", max + 1)), "{err}");
    }

    #[test]
    fn a_runtime_directory_of_this_user_alone_is_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("tilr-proto-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join("run");
        assert!(check_runtime_dir(&dir).is_ok(), "not there yet");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(check_runtime_dir(&dir).is_ok());

        for mode in [0o750, 0o705, 0o755, 0o770] {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
            let err = check_runtime_dir(&dir).unwrap_err();
            assert!(err.contains("open to other users"), "{mode:o}: {err}");
            assert!(err.contains(&format!("{mode:04o}")), "{err}");
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let err = check_runtime_dir(&link).unwrap_err();
        assert!(err.contains("symbolic link"), "{err}");

        let file = root.join("file");
        std::fs::write(&file, "").unwrap();
        let err = check_runtime_dir(&file).unwrap_err();
        assert!(err.contains("not a directory"), "{err}");

        // Another user's: the owner cannot be changed without root, the
        // check is given another uid instead.
        let meta = std::fs::symlink_metadata(&dir).unwrap();
        let err = check_dir_meta(&dir, &meta, meta.uid() + 1).unwrap_err();
        assert!(
            err.contains(&format!("belongs to another user (uid {})", meta.uid())),
            "{err}"
        );
        assert!(err.contains(RUNTIME_DIR_ENV), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
