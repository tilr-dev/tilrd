//! Process fixture for scripts/test_server_cleanup_stand.py.
//! Run only through that script: it deliberately leaves the test process blocked.

#[cfg(unix)]
mod common;

#[cfg(unix)]
#[test]
#[ignore = "requires the external cleanup stand"]
fn server_ready_then_wait_for_interruption() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixDatagram;

    let dir = std::path::PathBuf::from(
        std::env::var_os("TILR_STAND_DIR").expect("run through the external cleanup stand"),
    );
    let notify_path = dir.join("notify.sock");
    let notify = UnixDatagram::bind(&notify_path).unwrap();
    notify.set_read_timeout(Some(common::TIMEOUT)).unwrap();
    let path = notify_path.to_str().unwrap();
    let server = common::Server::start_with_env(dir, &[], &[("NOTIFY_SOCKET", path)]);
    let mut message = [0; 64];
    let n = notify.recv(&mut message).expect("server READY=1 event");
    assert_eq!(&message[..n], b"READY=1");

    println!("TILR_STAND_READY pid={}", server.child.id());
    std::io::stdout().flush().unwrap();
    let mut release = [0];
    std::io::stdin().read_exact(&mut release).unwrap();
    assert_eq!(release, [b'R']);
    drop(server);
}
