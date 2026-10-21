//! Internal integration-harness hook. No ownership connection in normal use.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

pub fn watch() -> anyhow::Result<()> {
    let Some(path) = std::env::var_os("TILR_TEST_OWNER_SOCKET") else {
        return Ok(());
    };
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(&std::process::id().to_ne_bytes())?;
    let mut ready = [0];
    stream.read_exact(&mut ready)?;
    anyhow::ensure!(ready == *b"r", "invalid test owner acknowledgement");
    std::thread::Builder::new()
        .name("test-owner".into())
        .spawn(move || {
            let mut byte = [0];
            loop {
                match stream.read(&mut byte) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    _ => break,
                }
            }
            // Use the ordinary shutdown path, which also stops panes and shells.
            // SAFETY: getpid and kill have no memory-safety preconditions.
            unsafe {
                libc::kill(libc::getpid(), libc::SIGTERM);
            }
        })?;
    Ok(())
}
