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

use std::ffi::OsString;
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
}
