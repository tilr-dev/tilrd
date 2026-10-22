//! One connection to tilrd: the WebSocket on its Unix socket or TCP port,
//! `Hello`, and the requests `tilr` makes.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, ClosePane, CreatePane, ExecCancel, ExecStart, Hello, ListPanes,
    PROTOCOL_VERSION, PaneInfo, ServerMessage, client_message::Msg, server_message,
};
use tokio::net::{TcpStream, UnixStream};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::target::Target;

/// How long connecting and saying hello may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a request waits for its answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

enum Ws {
    Unix(Box<WebSocketStream<UnixStream>>),
    Tcp(Box<WebSocketStream<MaybeTlsStream<TcpStream>>>),
}

pub struct Conn {
    ws: Ws,
    next_request: u64,
    /// When anything last came from the server, pongs included.
    pub heard: Instant,
    /// What the server said it is, in `HelloAck`.
    pub server_version: String,
    pub applied_input: Vec<tilr_proto::InputAck>,
}

/// How a server command ended.
pub struct CommandExit {
    pub code: i32,
    pub signal: i32,
    /// Why the server ended it itself ("reload", "shutdown"), if it did.
    pub reason: String,
}

/// Why a connection could not be made.
#[derive(Debug)]
pub enum ConnectError {
    /// Nothing listens there: no socket, or one nobody accepts on.
    NoServer(std::io::Error),
    /// Something answered, but not as tilrd does, or turned this client away.
    Other(anyhow::Error),
    /// Not talked to: the runtime directory is not this user's alone, or
    /// another user serves the socket.
    Untrusted(anyhow::Error),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer(err) => write!(f, "{err}"),
            Self::Other(err) | Self::Untrusted(err) => write!(f, "{err:#}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// The server closed the connection with a WebSocket close frame; the close
/// code says why (`tilr_proto::close`).
#[derive(Debug)]
pub struct Closed {
    /// None: a close frame without a code.
    pub code: Option<u16>,
    pub reason: String,
}

impl Closed {
    /// A newer connection of this client id took over: not to be made again.
    pub fn superseded(&self) -> bool {
        self.code == Some(tilr_proto::close::SUPERSEDED)
    }
}

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.reason.is_empty() {
            write!(f, "the server closed the connection")
        } else {
            write!(f, "the server closed the connection: {}", self.reason)
        }
    }
}

impl std::error::Error for Closed {}

impl Conn {
    /// Connect and say hello as `client_id`.
    pub async fn connect(target: &Target, client_id: &str) -> Result<Self, ConnectError> {
        let attempt = async {
            let (ws, token) = match target {
                Target::Local(path) => (Ws::Unix(Box::new(open_local(path).await?)), String::new()),
                Target::Remote { url, token } => {
                    (Ws::Tcp(Box::new(open_remote(url).await?)), token.clone())
                }
            };
            let mut conn = Self {
                ws,
                next_request: 0,
                heard: Instant::now(),
                server_version: String::new(),
                applied_input: Vec::new(),
            };
            conn.hello(client_id, token)
                .await
                .map_err(ConnectError::Other)?;
            Ok(conn)
        };
        match tokio::time::timeout(CONNECT_TIMEOUT, attempt).await {
            Ok(result) => result,
            Err(_) => Err(ConnectError::Other(anyhow!("{target} did not answer"))),
        }
    }

    async fn hello(&mut self, client_id: &str, token: String) -> Result<()> {
        self.send(Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "tilr".into(),
            // `tilr attach` writes the copies made in its pane to its
            // terminal; other commands watch no pane, so get none.
            features: vec![crate::attach::FEATURE_CLIPBOARD_WRITE.into()],
            token,
            client_id: client_id.to_owned(),
        }))
        .await?;
        match self.recv().await? {
            server_message::Msg::HelloAck(ack) if ack.protocol_version == PROTOCOL_VERSION => {
                self.server_version = ack.server_version;
                self.applied_input = ack.applied_input;
                Ok(())
            }
            server_message::Msg::HelloAck(ack) => bail!(
                "the server speaks protocol {}, tilr {PROTOCOL_VERSION}",
                ack.protocol_version
            ),
            server_message::Msg::Error(err) => bail!("{}", err.message),
            other => bail!("expected HelloAck, got {other:?}"),
        }
    }

    pub async fn send(&mut self, msg: Msg) -> Result<()> {
        self.send_message(ClientMessage::from(msg)).await
    }

    pub async fn send_message(&mut self, msg: ClientMessage) -> Result<()> {
        let message = Message::Binary(tilr_proto::encode(&msg));
        match &mut self.ws {
            Ws::Unix(ws) => ws.send(message).await,
            Ws::Tcp(ws) => ws.send(message).await,
        }
        .context("send to the server")
    }

    /// A WebSocket ping: the answer moves [`Conn::heard`].
    pub async fn ping(&mut self) -> Result<()> {
        let message = Message::Ping(Default::default());
        match &mut self.ws {
            Ws::Unix(ws) => ws.send(message).await,
            Ws::Tcp(ws) => ws.send(message).await,
        }
        .context("send to the server")
    }

    /// The next message from the server, server errors included.
    ///
    /// Cancel-safe: dropping the future loses no message.
    pub async fn recv(&mut self) -> Result<server_message::Msg> {
        loop {
            let message = match &mut self.ws {
                Ws::Unix(ws) => ws.next().await,
                Ws::Tcp(ws) => ws.next().await,
            };
            let message = message
                .context("the server closed the connection")?
                .context("read from the server")?;
            self.heard = Instant::now();
            let packet = match message {
                Message::Binary(packet) => packet,
                Message::Close(frame) => {
                    return Err(Closed {
                        code: frame.as_ref().map(|frame| u16::from(frame.code)),
                        reason: frame
                            .map(|frame| frame.reason.to_string())
                            .unwrap_or_default(),
                    }
                    .into());
                }
                _ => continue,
            };
            if let Some(msg) = tilr_proto::decode::<ServerMessage>(&packet)?.msg {
                return Ok(msg);
            }
        }
    }

    /// Wait for the answer a request names; server errors on the way fail it.
    async fn answer<T>(
        &mut self,
        mut pick: impl FnMut(server_message::Msg) -> Option<T>,
    ) -> Result<T> {
        let wait = async {
            loop {
                match self.recv().await? {
                    server_message::Msg::Error(err) => bail!("{}", err.message),
                    msg => {
                        if let Some(found) = pick(msg) {
                            return Ok(found);
                        }
                    }
                }
            }
        };
        tokio::time::timeout(REQUEST_TIMEOUT, wait)
            .await
            .context("the server did not answer")?
    }

    /// The panes, oldest first.
    pub async fn list_panes(&mut self) -> Result<Vec<PaneInfo>> {
        self.next_request += 1;
        let request_id = self.next_request;
        self.send(Msg::ListPanes(ListPanes { request_id })).await?;
        self.answer(|msg| match msg {
            server_message::Msg::PaneList(list) if list.request_id == request_id => {
                Some(list.panes)
            }
            _ => None,
        })
        .await
    }

    pub async fn create_pane(
        &mut self,
        cols: u16,
        rows: u16,
        command: Vec<String>,
        cwd: String,
    ) -> Result<PaneInfo> {
        self.next_request += 1;
        let request_id = self.next_request;
        self.send(Msg::CreatePane(CreatePane {
            cols: u32::from(cols),
            rows: u32::from(rows),
            command,
            cwd,
            request_id,
        }))
        .await?;
        self.answer(|msg| match msg {
            server_message::Msg::PaneCreated(created) if created.request_id == request_id => {
                Some(created.pane.unwrap_or_default())
            }
            _ => None,
        })
        .await
    }

    /// Close a pane and wait until it is gone.
    pub async fn close_pane(&mut self, pane_id: &str) -> Result<()> {
        self.send(Msg::ClosePane(ClosePane {
            pane_id: pane_id.to_owned(),
        }))
        .await?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        while Instant::now() < deadline {
            if !self
                .list_panes()
                .await?
                .iter()
                .any(|pane| pane.pane_id == pane_id)
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("pane {pane_id} is still there")
    }

    /// Run the server's own executable with `args` in the server (a server
    /// command: `token`, `cloud login`, `update`), its output going to ours
    /// as it comes. Ctrl+C cancels it.
    pub async fn server_command(&mut self, args: &[String]) -> Result<CommandExit> {
        use std::io::Write;
        self.next_request += 1;
        let request_id = self.next_request;
        self.send(Msg::ExecStart(ExecStart {
            request_id,
            argv: args.to_vec(),
            server_command: true,
            ..Default::default()
        }))
        .await?;
        let exec_id = self
            .answer(|msg| match msg {
                server_message::Msg::ExecStarted(started) if started.request_id == request_id => {
                    Some(started)
                }
                _ => None,
            })
            .await?;
        if !exec_id.error.is_empty() {
            bail!("{}", exec_id.error);
        }
        let exec_id = exec_id.exec_id;
        let mut cancelled = false;
        loop {
            let msg = tokio::select! {
                msg = self.recv() => msg?,
                _ = tokio::signal::ctrl_c(), if !cancelled => {
                    cancelled = true;
                    self.send(Msg::ExecCancel(ExecCancel { exec_id })).await?;
                    continue;
                }
            };
            match msg {
                server_message::Msg::ExecOutput(output) if output.exec_id == exec_id => {
                    if output.stream == 2 {
                        let _ = std::io::stderr().write_all(&output.data);
                    } else {
                        let mut out = std::io::stdout();
                        let _ = out.write_all(&output.data);
                        let _ = out.flush();
                    }
                }
                server_message::Msg::ExecExit(exit) if exit.exec_id == exec_id => {
                    return Ok(CommandExit {
                        code: exit.code,
                        signal: exit.signal,
                        reason: exit.reason,
                    });
                }
                server_message::Msg::Error(err) => bail!("{}", err.message),
                _ => {}
            }
        }
    }

    /// The pid of the server on the other end of a Unix socket.
    pub fn server_pid(&self) -> Option<i32> {
        match &self.ws {
            Ws::Unix(ws) => ws.get_ref().peer_cred().ok()?.pid(),
            Ws::Tcp(_) => None,
        }
    }
}

/// The local server's WebSocket. Nothing is sent before the socket is known
/// to be this user's: in the runtime directory (the default socket), the
/// directory must be private to the user; wherever it is, the process on the
/// other end must run as this user.
async fn open_local(path: &Path) -> Result<WebSocketStream<UnixStream>, ConnectError> {
    if tilr_proto::local::is_default(path)
        && let Some(dir) = path.parent()
    {
        tilr_proto::local::check_runtime_dir(dir)
            .map_err(|err| ConnectError::Untrusted(anyhow!(err)))?;
    }
    let stream = UnixStream::connect(path).await.map_err(|err| {
        use std::io::ErrorKind::{ConnectionRefused, NotFound};
        match err.kind() {
            NotFound | ConnectionRefused => ConnectError::NoServer(err),
            _ => ConnectError::Other(
                anyhow::Error::new(err).context(format!("connect to {}", path.display())),
            ),
        }
    })?;
    check_peer(&stream, path)?;
    let (ws, _) = tokio_tungstenite::client_async(tilr_proto::local::URL, stream)
        .await
        .map_err(|err| {
            ConnectError::Other(anyhow!(
                "{} does not answer as tilrd does: {err}",
                path.display()
            ))
        })?;
    Ok(ws)
}

/// The process on the other end of `stream` must run as this user.
fn check_peer(stream: &UnixStream, path: &Path) -> Result<(), ConnectError> {
    use std::os::fd::AsFd;
    tilr_proto::local::check_peer(stream.as_fd(), path)
        .map_err(|err| ConnectError::Untrusted(anyhow!(err)))
}

async fn open_remote(
    url: &str,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, ConnectError> {
    let (ws, _) = tilr_websocket::connect(url).await.map_err(|err| {
        use tokio_tungstenite::tungstenite::Error;
        match err {
            Error::Io(err) => ConnectError::NoServer(err),
            err => {
                ConnectError::Other(anyhow::Error::new(err).context(format!("connect to {url}")))
            }
        }
    })?;
    if let MaybeTlsStream::Plain(stream) = ws.get_ref() {
        let _ = stream.set_nodelay(true);
    }
    Ok(ws)
}

#[cfg(test)]
#[path = "../../tilr-websocket/tests/support/mod.rs"]
mod tls_test_peer;

#[cfg(test)]
mod bundled_tls_tests {
    #[tokio::test]
    async fn client_uses_the_bundled_trust_anchors() {
        let (url, server) = super::tls_test_peer::peer_with_bundled_issuer_name().await;
        let error = super::open_remote(&url).await.unwrap_err();
        server.await.unwrap();
        super::tls_test_peer::assert_bundled_root_was_used(error);
    }
}

#[cfg(test)]
mod update_wait_tests {
    use super::*;
    use tilr_proto::{ExecExit, ExecStarted};
    use tokio_tungstenite::tungstenite::protocol::Role;

    #[tokio::test]
    async fn update_waits_for_command_exit_after_a_long_launch_delay() {
        let (client, server) = UnixStream::pair().unwrap();
        let ws = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        let mut peer = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        let mut conn = Conn {
            ws: Ws::Unix(Box::new(ws)),
            next_request: 0,
            heard: Instant::now(),
            server_version: "test".into(),
        };
        let task = tokio::spawn(async move { conn.server_command(&["update".into()]).await });
        let packet = peer.next().await.unwrap().unwrap().into_data();
        let request = tilr_proto::decode::<ClientMessage>(&packet).unwrap();
        let Some(Msg::ExecStart(start)) = request.msg else {
            panic!("expected update start")
        };
        assert_eq!(start.argv, ["update"]);
        assert!(start.server_command);
        assert_eq!(start.timeout_ms, 0);
        peer.send(Message::Binary(tilr_proto::encode(&ServerMessage {
            msg: Some(server_message::Msg::ExecStarted(ExecStarted {
                request_id: start.request_id,
                exec_id: 1,
                ..Default::default()
            })),
        })))
        .await
        .unwrap();
        // A pong proves the client consumed ExecStarted and is waiting for exit.
        peer.send(Message::Ping(vec![1].into())).await.unwrap();
        assert!(matches!(
            peer.next().await.unwrap().unwrap(),
            Message::Pong(_)
        ));
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(86_400)).await;
        assert!(
            !task.is_finished(),
            "first launch must not consume the reconnect window"
        );
        peer.send(Message::Binary(tilr_proto::encode(&ServerMessage {
            msg: Some(server_message::Msg::ExecExit(ExecExit {
                exec_id: 1,
                code: 0,
                ..Default::default()
            })),
        })))
        .await
        .unwrap();
        let exit = task.await.unwrap().unwrap();
        assert_eq!(exit.code, 0);
        assert!(exit.reason.is_empty());
    }
}
