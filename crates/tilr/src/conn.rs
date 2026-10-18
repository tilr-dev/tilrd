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
            features: Vec::new(),
            token,
            client_id: client_id.to_owned(),
        }))
        .await?;
        match self.recv().await? {
            server_message::Msg::HelloAck(ack) if ack.protocol_version == PROTOCOL_VERSION => {
                self.server_version = ack.server_version;
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
        let message = Message::Binary(tilr_proto::encode(&ClientMessage::from(msg)));
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
                Message::Close(frame) => match frame {
                    Some(frame) => bail!("the server closed the connection: {}", frame.reason),
                    None => bail!("the server closed the connection"),
                },
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
    let cred = stream.peer_cred().map_err(|err| {
        ConnectError::Untrusted(anyhow::Error::new(err).context(format!(
            "who serves {}: no peer credentials",
            path.display()
        )))
    })?;
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if cred.uid() != uid {
        return Err(ConnectError::Untrusted(anyhow!(
            "socket {} is served by another user (uid {}), not by this one (uid {uid}): \
             nothing was sent",
            path.display(),
            cred.uid()
        )));
    }
    Ok(())
}

async fn open_remote(
    url: &str,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, ConnectError> {
    let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|err| {
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
