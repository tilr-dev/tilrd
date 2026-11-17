//! Harness for the bus (`tilr.bus.v1`): a real server, the owner's `tilr_bus::Bus`
//! clients, the test runtime (`crates/tilr-bus/examples/test_runtime.rs`) as a
//! WebSocket participant, and a raw client that speaks envelopes by hand.
//!
//! Owner: `bus-broker-core` (frozen after it, additive changes only). A lane's own
//! helpers go in `bus_<lane>.rs` next to this file.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tilr_bus::bus::{Envelope, Hello, HelloAck, MethodSpec, envelope::Msg};
use tilr_bus::{Bus, BusError, ConnectOptions, Credential, Transport, codec};
use tokio::net::UnixStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};

use super::{Server, TIMEOUT};

/// How a [`BusServer`] is started.
#[derive(Clone, Debug, Default)]
pub struct BusOptions {
    /// More arguments of `tilrd` (`--listen 127.0.0.1:0` is always given).
    pub args: Vec<String>,
    /// The text of `bus.toml` in the configuration directory.
    pub bus_toml: Option<String>,
    /// The text of `server.toml` in the configuration directory.
    pub server_toml: Option<String>,
    /// `TILR_FAILPOINTS` (src/fail.rs).
    pub failpoints: String,
    /// More environment of the server.
    pub env: Vec<(String, String)>,
}

/// A running server, for bus tests; derefs to the `tilr.v1` harness's [`Server`].
pub struct BusServer {
    pub server: Server,
}

impl std::ops::Deref for BusServer {
    type Target = Server;
    fn deref(&self) -> &Server {
        &self.server
    }
}

impl std::ops::DerefMut for BusServer {
    fn deref_mut(&mut self) -> &mut Server {
        &mut self.server
    }
}

/// How a participant on the TCP port presents itself.
#[derive(Clone, Debug)]
pub enum TcpAuth {
    /// As the owner, with this access token.
    AccessToken(String),
    /// As an outside participant, with a credential.
    Credential(Credential),
}

/// The test runtime, `<target>/<profile>/examples/test_runtime` beside `tilrd`
/// (`<target>/<profile>/tilrd`, wherever Cargo keeps this test's executable: in
/// `<profile>/deps/` before Cargo 1.100, under its build directory from 1.100 on).
/// A workspace test run builds it with tilr-bus's tests (`cargo nextest run
/// --workspace`); the test does not run cargo itself, whose builds from inside a
/// test see the test's environment and rebuild what the outer build made.
pub fn test_runtime_path() -> PathBuf {
    let profile_dir = Path::new(env!("CARGO_BIN_EXE_tilrd")).parent().unwrap();
    let name = format!("test_runtime{}", std::env::consts::EXE_SUFFIX);
    let path = profile_dir.join("examples").join(&name);
    assert!(
        path.is_file(),
        "no {}: cargo build -p tilr-bus --example test_runtime (a workspace test run builds it)",
        path.display()
    );
    path
}

impl BusServer {
    pub fn start(options: BusOptions) -> Self {
        let dir = Server::scratch_dir();
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        if let Some(text) = &options.bus_toml {
            std::fs::write(config.join("bus.toml"), text).unwrap();
        }
        if let Some(text) = &options.server_toml {
            std::fs::write(config.join("server.toml"), text).unwrap();
        }
        let args: Vec<&str> = options.args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = options
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let server = Server::start_full(
            dir,
            Path::new(env!("CARGO_BIN_EXE_tilrd")),
            &args,
            &options.failpoints,
            &env,
        );
        // The server binds its Unix socket before it logs `listening on`, which
        // `start_full` waited for: the socket takes connections from here on.
        let bus = Self { server };
        assert!(
            bus.socket().exists(),
            "no Unix socket at {}",
            bus.socket().display()
        );
        bus
    }

    /// The server's Unix socket.
    pub fn socket(&self) -> PathBuf {
        self.dir.join("runtime/tilrd.sock")
    }

    /// `ws://<addr>` of the TCP port.
    pub fn url(&self) -> String {
        self.addr.clone()
    }

    /// The owner on the Unix socket, providing nothing.
    pub async fn owner(&self) -> Bus {
        self.owner_with(ConnectOptions::new("owner-test")).await
    }

    /// The owner on the Unix socket, as `options` say.
    pub async fn owner_with(&self, options: ConnectOptions) -> Bus {
        self.try_owner_with(options)
            .await
            .expect("connect as owner")
    }

    pub async fn try_owner_with(&self, options: ConnectOptions) -> Result<Bus, BusError> {
        Bus::connect(
            Transport::UnixWebSocket {
                path: self.socket(),
                credential: None,
                origin: None,
            },
            options,
        )
        .await
    }

    /// A participant on the TCP port.
    pub async fn tcp(&self, auth: TcpAuth) -> Result<Bus, BusError> {
        self.tcp_with(auth, ConnectOptions::new("tcp-test")).await
    }

    pub async fn tcp_with(&self, auth: TcpAuth, options: ConnectOptions) -> Result<Bus, BusError> {
        let (access_token, credential) = match auth {
            TcpAuth::AccessToken(token) => (Some(token), None),
            TcpAuth::Credential(credential) => (None, Some(credential)),
        };
        Bus::connect(
            Transport::WebSocket {
                url: self.url(),
                access_token,
                credential,
                origin: None,
            },
            options,
        )
        .await
    }

    /// The test runtime as a WebSocket participant on the Unix socket (the owner,
    /// told apart by `client_name`), with `env` added; returns once it is greeted.
    pub async fn test_runtime(&self, client_name: &str, env: &[(&str, &str)]) -> TestRuntime {
        let greeted = self.greeted(client_name);
        let mut command = Command::new(test_runtime_path());
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TILR_BUS_URL", format!("unix:{}", self.socket().display()))
            .env("TEST_CLIENT_NAME", client_name)
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let child = command.spawn().expect("start the test runtime");
        let runtime = TestRuntime {
            child,
            name: client_name.to_owned(),
        };
        self.wait_greeted(client_name, greeted).await;
        runtime
    }

    /// How many times a participant named `client_name` was greeted so far.
    pub fn greeted(&self, client_name: &str) -> usize {
        let needle = format!("name={client_name:?}");
        self.log_lines()
            .iter()
            .filter(|line| line.contains("participant greeted") && line.contains(&needle))
            .count()
    }

    /// Wait until a participant named `client_name` is greeted again, `before`
    /// being [`greeted`](Self::greeted) earlier.
    pub async fn wait_greeted(&self, client_name: &str, before: usize) {
        self.wait_logged_event(&format!("{client_name} was not greeted"), |_| {
            self.greeted(client_name) > before
        })
        .await;
    }

    /// Wait up to [`TIMEOUT`] for the server's log so far to satisfy `done`, woken
    /// by each line it logs (`Server::log_changed`); fails with `what` and the log.
    pub async fn wait_logged_event(&self, what: &str, done: impl Fn(&[String]) -> bool) {
        let met = tokio::time::timeout(TIMEOUT, async {
            loop {
                let changed = self.log_changed.notified();
                tokio::pin!(changed);
                // Registered before the look, so a line logged in between wakes it.
                changed.as_mut().enable();
                if done(&self.log_lines()) {
                    return;
                }
                changed.await;
            }
        })
        .await;
        if met.is_err() {
            panic!("{what}:\n{}", self.log_lines().join("\n"));
        }
    }

    /// What the server logged so far.
    pub fn log_lines(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Reload the server (SIGUSR1) and wait until the successor serves.
    pub async fn reload(&self) {
        let resumed = |lines: &[String]| {
            lines
                .iter()
                .filter(|line| line.contains("resumed after reload"))
                .count()
        };
        let before = resumed(&self.log_lines());
        self.signal("-USR1");
        self.wait_logged_event("no reload", |lines| resumed(lines) > before)
            .await;
    }

    /// Kill the server outright.
    pub fn kill(&mut self) {
        let _ = self.server.child.kill();
        let _ = self.server.child.wait();
    }

    /// A raw client on the Unix socket that offered `tilr.bus.v1` and has said
    /// nothing yet.
    pub async fn raw(&self) -> Raw {
        Raw::connect_unix(&self.socket()).await
    }

    /// A raw client, greeted as the owner, providing `provides`.
    pub async fn raw_greeted(&self, client_name: &str, provides: Vec<MethodSpec>) -> Raw {
        let mut raw = self.raw().await;
        raw.send(Msg::Hello(Hello {
            protocol_version: tilr_bus::PROTOCOL_VERSION,
            client_name: client_name.into(),
            provides,
            ..Hello::default()
        }))
        .await;
        match raw.recv().await {
            Some(Msg::HelloAck(ack)) => raw.ack = Some(ack),
            other => panic!("{client_name}: expected HelloAck, got {other:?}"),
        }
        raw
    }
}

/// The test runtime process; killed when dropped.
pub struct TestRuntime {
    pub child: Child,
    pub name: String,
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A raw bus client: envelopes and WebSocket messages by hand.
pub struct Raw {
    pub ws: WebSocketStream<UnixStream>,
    /// The `HelloAck`, once greeted by [`BusServer::raw_greeted`].
    pub ack: Option<HelloAck>,
    /// How the server closed the connection, once it did.
    pub close: Option<CloseFrame>,
}

impl Raw {
    pub async fn connect_unix(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).await.expect("connect");
        let mut request = "ws://localhost/".into_client_request().unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_static(tilr_bus::SUBPROTOCOL),
        );
        let max = codec::MAX_FRAME + (1 << 20);
        let config = WebSocketConfig::default()
            .max_message_size(Some(max))
            .max_frame_size(Some(max));
        let (ws, response) =
            tokio_tungstenite::client_async_with_config(request, stream, Some(config))
                .await
                .expect("handshake");
        assert_eq!(
            response
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|value| value.to_str().ok()),
            Some(tilr_bus::SUBPROTOCOL),
            "the server agrees on the subprotocol"
        );
        Self {
            ws,
            ack: None,
            close: None,
        }
    }

    pub async fn send(&mut self, msg: Msg) {
        let envelope = Envelope { msg: Some(msg) };
        self.send_raw(codec::encode_ws(&envelope).unwrap().to_vec())
            .await;
    }

    /// One binary WebSocket message as is.
    pub async fn send_raw(&mut self, message: Vec<u8>) {
        self.ws.send(Message::Binary(message.into())).await.unwrap();
    }

    /// The next envelope, `None` once the connection closed; waits up to
    /// [`TIMEOUT`].
    pub async fn recv(&mut self) -> Option<Msg> {
        tokio::time::timeout(TIMEOUT, self.recv_inner())
            .await
            .expect("nothing from the server in time")
    }

    /// The next envelope within `wait`; `None` if none came (or it closed).
    pub async fn recv_within(&mut self, wait: Duration) -> Option<Msg> {
        tokio::time::timeout(wait, self.recv_inner())
            .await
            .ok()
            .flatten()
    }

    async fn recv_inner(&mut self) -> Option<Msg> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Binary(message))) => {
                    let envelope = codec::decode_ws(&message).expect("the server's envelope");
                    match envelope.msg {
                        Some(msg) => return Some(msg),
                        None => continue,
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    self.close = frame;
                    return None;
                }
                Some(Ok(_)) => continue,
                None | Some(Err(_)) => return None,
            }
        }
    }

    /// Wait for the next envelope `pick` takes, skipping others.
    pub async fn wait_for<T>(&mut self, mut pick: impl FnMut(&Msg) -> Option<T>) -> T {
        loop {
            match self.recv().await {
                Some(msg) => {
                    if let Some(found) = pick(&msg) {
                        return found;
                    }
                }
                None => panic!("closed while waiting"),
            }
        }
    }

    /// Whether the connection closes (with nothing else but `skip`ped envelopes).
    pub async fn closes(&mut self) -> bool {
        loop {
            match self.recv().await {
                None => return true,
                Some(Msg::ConnectionError(_)) => continue,
                Some(_) => return false,
            }
        }
    }
}

/// A text payload, as the test runtime takes and answers them.
pub fn text(payload: &Bytes) -> String {
    String::from_utf8_lossy(payload).into_owned()
}
