//! Use cases: the SOCKS proxy, which lets a browser elsewhere reach what
//! listens on the server's machine. Over the WebSocket at `/socks` (the way
//! through a tunnel) and as raw SOCKS5 on the same port.

mod common;

use common::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// One end of a SOCKS conversation, whatever carries it.
trait Wire {
    async fn send(&mut self, bytes: &[u8]);
    /// Exactly `n` bytes; `None` if the server closed before.
    async fn recv(&mut self, n: usize) -> Option<Vec<u8>>;
}

impl Wire for TcpStream {
    async fn send(&mut self, bytes: &[u8]) {
        self.write_all(bytes).await.unwrap();
    }

    async fn recv(&mut self, n: usize) -> Option<Vec<u8>> {
        let mut bytes = vec![0; n];
        tokio::time::timeout(TIMEOUT, self.read_exact(&mut bytes))
            .await
            .expect("server answered in time")
            .ok()?;
        Some(bytes)
    }
}

/// SOCKS over the WebSocket: bytes go in binary messages of any size.
struct WsWire {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    unread: VecDeque<u8>,
    /// The close the server sent, once it has.
    closed: Option<Option<u16>>,
}

impl Wire for WsWire {
    async fn send(&mut self, bytes: &[u8]) {
        self.ws
            .send(Message::Binary(Bytes::copy_from_slice(bytes)))
            .await
            .unwrap();
    }

    async fn recv(&mut self, n: usize) -> Option<Vec<u8>> {
        while self.unread.len() < n {
            let message = tokio::time::timeout(TIMEOUT, self.ws.next())
                .await
                .expect("server answered in time");
            match message {
                Some(Ok(Message::Binary(bytes))) => self.unread.extend(bytes),
                Some(Ok(Message::Close(frame))) => {
                    self.closed = Some(frame.map(|frame| u16::from(frame.code)));
                    return None;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    self.closed.get_or_insert(None);
                    return None;
                }
            }
        }
        Some(self.unread.drain(..n).collect())
    }
}

/// Where a SOCKS client asks to go.
enum To<'a> {
    Ip(std::net::SocketAddr),
    Name(&'a str, u16),
}

const NO_AUTH: u8 = 0x00;
const PASSWORD: u8 = 0x02;
const CONNECT: u8 = 0x01;
const BIND: u8 = 0x02;

/// The greeting offering `methods`: the method the server picked.
async fn greet(wire: &mut impl Wire, methods: &[u8]) -> u8 {
    let mut greeting = vec![5, methods.len() as u8];
    greeting.extend(methods);
    wire.send(&greeting).await;
    let answer = wire.recv(2).await.expect("method selection");
    assert_eq!(answer[0], 5);
    answer[1]
}

/// RFC 1929: the status, 0 for let in.
async fn log_in(wire: &mut impl Wire, user: &str, password: &str) -> u8 {
    let mut message = vec![1, user.len() as u8];
    message.extend(user.as_bytes());
    message.push(password.len() as u8);
    message.extend(password.as_bytes());
    wire.send(&message).await;
    let answer = wire.recv(2).await.expect("password status");
    assert_eq!(answer[0], 1);
    answer[1]
}

/// A request: the reply code, 0 for connected.
async fn request(wire: &mut impl Wire, command: u8, to: To<'_>) -> u8 {
    let mut message = vec![5, command, 0];
    let port = match to {
        To::Ip(std::net::SocketAddr::V4(addr)) => {
            message.push(1);
            message.extend(addr.ip().octets());
            addr.port()
        }
        To::Ip(std::net::SocketAddr::V6(addr)) => {
            message.push(4);
            message.extend(addr.ip().octets());
            addr.port()
        }
        To::Name(name, port) => {
            message.extend([3, name.len() as u8]);
            message.extend(name.as_bytes());
            port
        }
    };
    message.extend(port.to_be_bytes());
    wire.send(&message).await;
    let head = wire.recv(4).await.expect("reply");
    assert_eq!(head[0], 5);
    let rest = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        kind => panic!("reply with address type {kind}"),
    };
    wire.recv(rest).await.expect("bound address");
    head[1]
}

/// Sends `bytes` through a connected session and expects them back.
async fn echoes(wire: &mut impl Wire, bytes: &[u8]) {
    wire.send(bytes).await;
    assert_eq!(wire.recv(bytes.len()).await.as_deref(), Some(bytes));
}

/// A service on the server's loopback that sends back what it gets.
async fn echo_service() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut from, mut to) = stream.split();
                let _ = tokio::io::copy(&mut from, &mut to).await;
            });
        }
    });
    addr
}

/// A loopback port nothing listens on.
async fn closed_port() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap()
}

fn socks_server(args: &[&str]) -> Server {
    let mut all = vec!["--socks"];
    all.extend(args);
    Server::start(&all)
}

/// Opens the SOCKS WebSocket: `query` goes after `/socks`, `headers` with
/// the handshake. The error is the HTTP answer.
async fn open(server: &Server, query: &str, headers: &[(&str, &str)]) -> Result<WsWire, String> {
    let url = format!("{}/socks{query}", server.addr);
    let mut request = url.into_client_request().unwrap();
    for (name, value) in headers {
        let name: tokio_tungstenite::tungstenite::http::HeaderName = name.parse().unwrap();
        request.headers_mut().insert(name, value.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((ws, _)) => Ok(WsWire {
            ws,
            unread: VecDeque::new(),
            closed: None,
        }),
        Err(err) => Err(err.to_string()),
    }
}

async fn open_with_token(server: &Server) -> WsWire {
    let bearer = format!("Bearer {}", server.token());
    open(server, "", &[("authorization", &bearer)])
        .await
        .expect("SOCKS WebSocket")
}

/// The listening port as a plain TCP address.
async fn raw(server: &Server) -> TcpStream {
    TcpStream::connect(server.addr.trim_start_matches("ws://"))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_page_reaches_a_loopback_service_through_the_socks_websocket() {
    let server = socks_server(&[]);
    let service = echo_service().await;
    assert!(server.logged("SOCKS proxy on"));

    let mut wire = open_with_token(&server).await;
    assert_eq!(greet(&mut wire, &[NO_AUTH]).await, NO_AUTH);
    assert_eq!(request(&mut wire, CONNECT, To::Ip(service)).await, 0);
    echoes(&mut wire, b"hello").await;
    // More than one message's worth, in both directions.
    let big: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
    echoes(&mut wire, &big).await;
}

#[tokio::test]
async fn a_page_that_cannot_set_headers_gives_the_token_in_the_query() {
    let server = socks_server(&[]);
    let service = echo_service().await;

    let query = format!("?token={}", server.token());
    let mut wire = open(&server, &query, &[]).await.unwrap();
    assert_eq!(greet(&mut wire, &[NO_AUTH]).await, NO_AUTH);
    // Names are resolved on the server.
    assert_eq!(
        request(&mut wire, CONNECT, To::Name("localhost", service.port())).await,
        0
    );
    echoes(&mut wire, b"by name").await;
}

#[tokio::test]
async fn any_name_under_localhost_is_the_servers_loopback() {
    // A browser goes to `localhost` itself, not through the proxy, so a page
    // names the server's loopback otherwise. Such a name is never looked up
    // in DNS: the one below exists nowhere.
    let server = socks_server(&[]);
    let service = echo_service().await;
    for name in ["pb.localhost", "nonexistent.LocalHost"] {
        let mut wire = open_with_token(&server).await;
        assert_eq!(greet(&mut wire, &[NO_AUTH]).await, NO_AUTH);
        assert_eq!(
            request(&mut wire, CONNECT, To::Name(name, service.port())).await,
            0,
            "{name}"
        );
        echoes(&mut wire, name.as_bytes()).await;
    }
}

#[tokio::test]
async fn the_socks_websocket_needs_the_access_token() {
    let server = socks_server(&[]);
    let refused = open(&server, "", &[]).await;
    assert!(refused.is_err_and(|err| err.contains("401")));
    let refused = open(&server, "", &[("authorization", "Bearer nope")]).await;
    assert!(refused.is_err_and(|err| err.contains("401")));
    let refused = open(&server, "?token=nope", &[]).await;
    assert!(refused.is_err_and(|err| err.contains("401")));

    // Without authentication, without the token.
    let server = socks_server(&["--no-auth"]);
    let service = echo_service().await;
    let mut wire = open(&server, "", &[]).await.unwrap();
    assert_eq!(greet(&mut wire, &[NO_AUTH]).await, NO_AUTH);
    assert_eq!(request(&mut wire, CONNECT, To::Ip(service)).await, 0);
    echoes(&mut wire, b"open").await;
}

#[tokio::test]
async fn the_socks_websocket_comes_only_from_an_allowed_origin() {
    let server = socks_server(&["--allow-origin", "http://app.test"]);
    let bearer = format!("Bearer {}", server.token());
    let refused = open(
        &server,
        "",
        &[("authorization", &bearer), ("origin", "http://evil.test")],
    )
    .await;
    assert!(refused.is_err_and(|err| err.contains("403")));
    assert!(
        open(
            &server,
            "",
            &[("authorization", &bearer), ("origin", "http://app.test")],
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn the_proxy_is_off_unless_asked_for() {
    let server = Server::start(&[]);
    assert!(server.logged("SOCKS proxy off"));
    let bearer = format!("Bearer {}", server.token());
    let refused = open(&server, "", &[("authorization", &bearer)]).await;
    assert!(refused.is_err_and(|err| err.contains("404")));

    // A raw SOCKS client is hung up on.
    let mut stream = raw(&server).await;
    stream.write_all(&[5, 1, PASSWORD]).await.unwrap();
    let mut answer = Vec::new();
    let read = tokio::time::timeout(TIMEOUT, stream.read_to_end(&mut answer)).await;
    assert!(read.is_ok_and(|read| read.is_err() || answer.is_empty()));

    // The main protocol is as it was.
    let mut client = Client::connect(&server).await;
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn the_configuration_file_turns_the_proxy_on() {
    let dir = Server::scratch_dir();
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/server.toml"), "socks = true\n").unwrap();
    let server = Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), &[]);
    assert!(server.logged("SOCKS proxy on"));
    let mut wire = open_with_token(&server).await;
    assert_eq!(greet(&mut wire, &[NO_AUTH]).await, NO_AUTH);
}

#[tokio::test]
async fn a_raw_socks_client_gives_the_token_as_its_password() {
    let server = socks_server(&[]);
    let service = echo_service().await;

    let mut stream = raw(&server).await;
    assert_eq!(greet(&mut stream, &[NO_AUTH, PASSWORD]).await, PASSWORD);
    assert_eq!(log_in(&mut stream, "anyone", &server.token()).await, 0);
    assert_eq!(request(&mut stream, CONNECT, To::Ip(service)).await, 0);
    echoes(&mut stream, b"raw").await;
    // The target's end closing reaches the client.
    stream.shutdown().await.unwrap();
    assert!(stream.recv(1).await.is_none());

    let mut stream = raw(&server).await;
    assert_eq!(greet(&mut stream, &[PASSWORD]).await, PASSWORD);
    assert_eq!(log_in(&mut stream, "anyone", "wrong").await, 1);
    assert!(stream.recv(1).await.is_none());

    // Without a password there is no way in.
    let mut stream = raw(&server).await;
    assert_eq!(greet(&mut stream, &[NO_AUTH]).await, 0xff);
    assert!(stream.recv(1).await.is_none());

    // The main protocol shares the port.
    let mut client = Client::connect(&server).await;
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn a_raw_socks_client_of_a_server_without_authentication_needs_no_password() {
    let server = socks_server(&["--no-auth"]);
    let service = echo_service().await;
    let mut stream = raw(&server).await;
    assert_eq!(greet(&mut stream, &[NO_AUTH]).await, NO_AUTH);
    assert_eq!(request(&mut stream, CONNECT, To::Ip(service)).await, 0);
    echoes(&mut stream, b"open").await;
}

#[tokio::test]
async fn only_connect_is_served_and_a_refusal_is_reported() {
    let server = socks_server(&[]);
    let service = echo_service().await;

    let mut wire = open_with_token(&server).await;
    greet(&mut wire, &[NO_AUTH]).await;
    assert_eq!(request(&mut wire, BIND, To::Ip(service)).await, 0x07);

    let mut wire = open_with_token(&server).await;
    greet(&mut wire, &[NO_AUTH]).await;
    let closed = closed_port().await;
    assert_eq!(request(&mut wire, CONNECT, To::Ip(closed)).await, 0x05);
    // Then the WebSocket is closed.
    assert!(wire.recv(1).await.is_none());
    assert_eq!(wire.closed, Some(Some(1000)));
}

#[tokio::test]
async fn a_text_message_closes_the_socks_websocket() {
    let server = socks_server(&[]);
    let mut wire = open_with_token(&server).await;
    wire.ws.send(Message::Text("hello".into())).await.unwrap();
    assert!(wire.recv(1).await.is_none());
    assert_eq!(wire.closed, Some(Some(1003)));
}

#[tokio::test]
async fn socks_sessions_run_side_by_side_with_the_terminal() {
    let server = socks_server(&[]);
    let service = echo_service().await;

    let mut first = open_with_token(&server).await;
    greet(&mut first, &[NO_AUTH]).await;
    assert_eq!(request(&mut first, CONNECT, To::Ip(service)).await, 0);
    let mut second = open_with_token(&server).await;
    greet(&mut second, &[NO_AUTH]).await;
    assert_eq!(request(&mut second, CONNECT, To::Ip(service)).await, 0);
    // A session that has yet to say where it goes holds nothing up.
    let mut idle = open_with_token(&server).await;
    idle.send(&[5]).await;

    let mut client = Client::connect(&server).await;
    assert!(client.list_panes().await.is_empty());
    echoes(&mut second, b"second").await;
    echoes(&mut first, b"first").await;
}
