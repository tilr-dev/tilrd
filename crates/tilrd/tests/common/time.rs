//! The test clock the server reads under `failpoints` (`TILR_TEST_TIME_FILE`).

/// Publish `us` (Unix microseconds) as the servers' test clock: a rename, so
/// a reader (the server or a pane process) sees one whole value. The next
/// protocol response is the test's ordering barrier.
pub fn set_test_time(path: &std::path::Path, us: u64) {
    let next = path.with_extension("next");
    std::fs::write(&next, us.to_string()).unwrap();
    std::fs::rename(next, path).unwrap();
}

/// The readiness log must be observed before calling this helper. A byte
/// confirms the test-clock path rather than the production timeout fired.
pub async fn signal_deadline(path: &std::path::Path, name: &str) {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(super::TIMEOUT, async {
        let port: u16 = std::fs::read_to_string(path.with_extension(name))
            .unwrap()
            .parse()
            .unwrap();
        let mut signal = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut ack = [0];
        signal
            .read_exact(&mut ack)
            .await
            .expect("test-clock deadline acknowledgement");
        assert_eq!(ack, [1]);
    })
    .await
    .expect("test-clock deadline signal timed out");
}

pub fn hello_deadline(client: &super::Client) -> String {
    let tokio_tungstenite::MaybeTlsStream::Plain(stream) = client.ws.get_ref() else {
        panic!("test connection must use plain TCP");
    };
    format!("hello-{}", stream.local_addr().unwrap().port())
}

/// A wake-up before the deadline is processed without acknowledging expiry.
pub async fn signal_before_deadline(path: &std::path::Path, name: &str) {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(super::TIMEOUT, async {
        let port: u16 = std::fs::read_to_string(path.with_extension(name))
            .unwrap()
            .parse()
            .unwrap();
        let mut signal = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut byte = [0];
        assert_eq!(signal.read(&mut byte).await.unwrap(), 0);
    })
    .await
    .expect("early test-clock signal timed out");
}
