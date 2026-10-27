//! Plain HTTP against the server's port, for `GET /metrics` and its scrape.

use super::{Server, TIMEOUT};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A plain HTTP request for `target` on the server's port, with `headers`:
/// the status and the body.
/// `Host` is the server's address unless `headers` name one.
pub async fn http_get(server: &Server, target: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let addr = server.addr.trim_start_matches("ws://");
    let mut request = format!("GET {target} HTTP/1.1\r\n");
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        request.push_str(&format!("Host: {addr}\r\n"));
    }
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    http_request(server, &[&request]).await
}

/// Send a request in `pieces`, a moment apart, and read the answer: the
/// status and the body.
pub async fn http_request(server: &Server, pieces: &[&str]) -> (u16, String) {
    let addr = server.addr.trim_start_matches("ws://");
    let mut stream = TcpStream::connect(addr).await.unwrap();
    for (i, piece) in pieces.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        stream.write_all(piece.as_bytes()).await.unwrap();
    }
    let mut response = Vec::new();
    tokio::time::timeout(TIMEOUT, stream.read_to_end(&mut response))
        .await
        .expect("server answered in time")
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").expect("a whole response");
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    if status == 200 {
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: text/plain; version=0.0.4"),
            "{head}"
        );
    }
    (status, body.to_owned())
}

/// The value of the sample `series` (name and labels, as written), if the
/// scrape has it.
pub fn sample(text: &str, series: &str) -> Option<f64> {
    text.lines()
        .find_map(|line| line.strip_prefix(series)?.strip_prefix(' '))
        .map(|value| value.parse().unwrap())
}

/// Scrape `GET /metrics` every 20 ms until the sample `series` is above
/// `floor`; fails if it is not within [`TIMEOUT`].
pub async fn wait_sample_above(server: &Server, series: &str, floor: f64) {
    let url = format!("/metrics?token={}", server.token());
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let (_, text) = http_get(server, &url, &[]).await;
        if sample(&text, series).is_some_and(|value| value > floor) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{series} not above {floor} within {TIMEOUT:?}:\n{text}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
