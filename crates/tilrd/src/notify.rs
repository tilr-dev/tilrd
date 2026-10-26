//! Telling a service manager the server is ready, as systemd's
//! `Type=notify` waits for (`tilr service install` writes such a unit):
//! `READY=1` to the datagram socket named by `$NOTIFY_SOCKET` once the
//! server listens. Without the variable, nothing. A reload keeps the pid and
//! says it again, which systemd takes as it is.

/// Say the server is ready. A failure is only logged: the server works on.
pub fn ready() {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    if let Err(err) = send(&path, b"READY=1") {
        tracing::warn!("could not tell the service manager the server is ready: {err}");
    }
}

#[cfg(target_os = "linux")]
fn send(path: &std::ffi::OsStr, message: &[u8]) -> std::io::Result<()> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::{SocketAddr, UnixDatagram};

    let socket = UnixDatagram::unbound()?;
    let bytes = path.as_bytes();
    // `@name`: a socket in the abstract namespace.
    let addr = match bytes.strip_prefix(b"@") {
        Some(name) => SocketAddr::from_abstract_name(name)?,
        None => SocketAddr::from_pathname(path)?,
    };
    socket.send_to_addr(message, &addr)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn send(path: &std::ffi::OsStr, message: &[u8]) -> std::io::Result<()> {
    std::os::unix::net::UnixDatagram::unbound()?.send_to(message, path)?;
    Ok(())
}
