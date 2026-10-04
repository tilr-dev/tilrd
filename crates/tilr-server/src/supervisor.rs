//! Supervisor: runs one pane process per pane and routes packets between
//! WebSocket clients and pane processes. The pty and the shell belong to the
//! pane process, so a pane lives exactly as long as its process.
//!
//! All state lives in one actor task ([`Supervisor::run`]); connection and
//! pane tasks only talk to it through [`Event`]s.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, CreatePane, Error as ProtoError, HelloAck, PROTOCOL_VERSION, PaneCreated,
    PaneExited, PaneInfo, PaneList, PaneStatus, ServerMessage, Subscribe, client_message::Msg, server_message,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::link;

/// Packets queued for one client before it is considered stuck and dropped.
const CLIENT_QUEUE: usize = 1024;

type ClientId = u64;
type PaneId = u64;

enum Event {
    ClientConnected {
        id: ClientId,
        tx: mpsc::Sender<Bytes>,
    },
    ClientPacket {
        id: ClientId,
        packet: Bytes,
    },
    ClientGone {
        id: ClientId,
    },
    /// The pane process has its shell running.
    PaneStarted {
        pane_id: PaneId,
    },
    /// A packet from a pane process, relayed to subscribers untouched.
    PaneOutput {
        pane_id: PaneId,
        packet: Bytes,
    },
    /// Title or working directory changed inside the pane.
    PaneStatus {
        pane_id: PaneId,
        status: PaneStatus,
    },
    /// The pane process is gone, and the shell with it. `exit_code` is the
    /// shell's, if the process got to report it; `error` is why the shell
    /// could not be started.
    PaneLinkClosed {
        pane_id: PaneId,
        exit_code: Option<i32>,
        error: Option<String>,
    },
    /// Have every pane process replace itself with the executable on disk.
    ReloadPanes,
    Shutdown,
}

struct Client {
    tx: mpsc::Sender<Bytes>,
    subscriptions: HashSet<PaneId>,
}

struct Pane {
    command: Vec<String>,
    title: String,
    cwd: String,
    created_at_ms: u64,
    cols: u16,
    rows: u16,
    subscribers: HashSet<ClientId>,
    to_pane: mpsc::UnboundedSender<Bytes>,
}

/// A pane whose process has not yet reported that the shell is running.
/// Clients do not see it until then.
struct Starting {
    pane: Pane,
    /// Who asked for the pane and gets the `PaneCreated` or the error.
    client_id: ClientId,
    request_id: u64,
}

pub fn run(listen: SocketAddr, allow_origins: Vec<String>) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let listener = TcpListener::bind(listen)
                .await
                .with_context(|| format!("bind {listen}"))?;
            tracing::info!("listening on ws://{}", listener.local_addr()?);

            let (events_tx, events_rx) = mpsc::unbounded_channel();
            tokio::spawn(accept_loop(listener, allow_origins.into(), events_tx.clone()));
            tokio::spawn(shutdown_signal(events_tx.clone()));
            tokio::spawn(reload_signal(events_tx.clone()));

            Supervisor {
                events: events_tx,
                clients: HashMap::new(),
                panes: HashMap::new(),
                starting: HashMap::new(),
                next_pane_id: 1,
            }
            .run(events_rx)
            .await;
            Ok(())
        })
}

async fn shutdown_signal(events: mpsc::UnboundedSender<Event>) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    let _ = events.send(Event::Shutdown);
}

/// SIGUSR1 reloads the pane processes, to pick up a rebuilt executable
/// without closing any shell.
async fn reload_signal(events: mpsc::UnboundedSender<Event>) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut usr1 = signal(SignalKind::user_defined1()).expect("install SIGUSR1 handler");
    while usr1.recv().await.is_some() {
        if events.send(Event::ReloadPanes).is_err() {
            break;
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    allow_origins: Arc<[String]>,
    events: mpsc::UnboundedSender<Event>,
) {
    let mut next_id: ClientId = 1;
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let id = next_id;
                next_id += 1;
                tokio::spawn(serve_client(
                    id,
                    stream,
                    addr,
                    Arc::clone(&allow_origins),
                    events.clone(),
                ));
            }
            Err(err) => {
                tracing::warn!("accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn serve_client(
    id: ClientId,
    stream: TcpStream,
    addr: SocketAddr,
    allow_origins: Arc<[String]>,
    events: mpsc::UnboundedSender<Event>,
) {
    // Keystrokes and small diffs must not wait for Nagle.
    let _ = stream.set_nodelay(true);
    // A browser attaches the page's origin to every WebSocket handshake. With
    // no other authentication, this check is what stops an arbitrary website
    // from opening a shell on this machine.
    #[expect(
        clippy::result_large_err,
        reason = "the handshake callback signature is fixed by tungstenite"
    )]
    let check_origin = |request: &Request, response: Response| match request
        .headers()
        .get("origin")
        .map(|origin| origin.to_str().unwrap_or(""))
    {
        Some(origin) if !allow_origins.iter().any(|allowed| allowed == origin) => {
            tracing::warn!(%addr, origin, "rejected connection from disallowed origin");
            let mut forbidden = ErrorResponse::new(Some("origin not allowed".into()));
            *forbidden.status_mut() = StatusCode::FORBIDDEN;
            Err(forbidden)
        }
        _ => Ok(response),
    };
    let ws = match tokio_tungstenite::accept_hdr_async(stream, check_origin).await {
        Ok(ws) => ws,
        Err(err) => {
            tracing::warn!(%addr, "websocket handshake failed: {err}");
            return;
        }
    };
    tracing::info!(client = id, %addr, "client connected");
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Bytes>(CLIENT_QUEUE);
    if events.send(Event::ClientConnected { id, tx }).is_err() {
        return;
    }

    // Ends when the supervisor drops the client's sender.
    let writer = tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            if sink.send(Message::Binary(packet)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(Ok(message)) = source.next().await {
        match message {
            Message::Binary(packet) => {
                if events.send(Event::ClientPacket { id, packet }).is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = events.send(Event::ClientGone { id });
    writer.abort();
    tracing::info!(client = id, "client disconnected");
}

struct Supervisor {
    events: mpsc::UnboundedSender<Event>,
    clients: HashMap<ClientId, Client>,
    panes: HashMap<PaneId, Pane>,
    starting: HashMap<PaneId, Starting>,
    next_pane_id: PaneId,
}

impl Supervisor {
    async fn run(mut self, mut events: mpsc::UnboundedReceiver<Event>) {
        while let Some(event) = events.recv().await {
            match event {
                Event::ClientConnected { id, tx } => {
                    self.clients.insert(
                        id,
                        Client {
                            tx,
                            subscriptions: HashSet::new(),
                        },
                    );
                }
                Event::ClientPacket { id, packet } => self.on_client_packet(id, packet),
                Event::ClientGone { id } => self.drop_client(id),
                Event::PaneStarted { pane_id } => self.on_pane_started(pane_id),
                Event::PaneOutput { pane_id, packet } => self.on_pane_output(pane_id, packet),
                Event::PaneStatus { pane_id, status } => {
                    if let Some(pane) = self.panes.get_mut(&pane_id) {
                        pane.title = status.title;
                        if !status.cwd.is_empty() {
                            pane.cwd = status.cwd;
                        }
                        self.broadcast_pane_list();
                    }
                }
                Event::PaneLinkClosed {
                    pane_id,
                    exit_code,
                    error,
                } => self.on_pane_link_closed(pane_id, exit_code, error),
                Event::ReloadPanes => {
                    tracing::info!("reloading {} pane process(es)", self.panes.len());
                    for pane in self.panes.values() {
                        // An empty packet is the reload request.
                        let _ = pane.to_pane.send(Bytes::new());
                    }
                }
                Event::Shutdown => break,
            }
        }
        // Returning drops the runtime and with it the pane processes; each
        // shell is hung up when its pty closes.
        tracing::info!("shutting down, closing {} pane(s)", self.panes.len());
    }

    fn on_client_packet(&mut self, client_id: ClientId, packet: Bytes) {
        let msg = match tilr_proto::decode::<ClientMessage>(&packet) {
            Ok(ClientMessage { msg: Some(msg) }) => msg,
            Ok(_) => return self.send_error(client_id, "empty message"),
            Err(err) => return self.send_error(client_id, &format!("bad packet: {err}")),
        };
        match msg {
            Msg::Hello(_) => self.send(
                client_id,
                server_message::Msg::HelloAck(HelloAck {
                    protocol_version: PROTOCOL_VERSION,
                    server_version: env!("CARGO_PKG_VERSION").into(),
                }),
            ),
            Msg::CreatePane(create) => {
                if let Err(err) = self.create_pane(client_id, &create) {
                    self.send_error(client_id, &format!("create pane: {err:#}"));
                }
            }
            Msg::ListPanes(list) => {
                let reply = self.pane_list(list.request_id);
                self.send(client_id, server_message::Msg::PaneList(reply));
            }
            // The pane process hangs up its shell and exits with it.
            Msg::ClosePane(close) => self.forward(client_id, close.pane_id, packet),
            Msg::Subscribe(Subscribe { pane_id }) => {
                let Some(pane) = self.panes.get_mut(&pane_id) else {
                    return self.send_no_pane(client_id, pane_id);
                };
                pane.subscribers.insert(client_id);
                // The pane answers with a full frame.
                let _ = pane.to_pane.send(packet);
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.subscriptions.insert(pane_id);
                }
                self.broadcast_pane_list();
            }
            Msg::Unsubscribe(unsubscribe) => {
                if let Some(pane) = self.panes.get_mut(&unsubscribe.pane_id) {
                    pane.subscribers.remove(&client_id);
                }
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.subscriptions.remove(&unsubscribe.pane_id);
                }
                self.broadcast_pane_list();
            }
            Msg::Resize(resize) => {
                let Some(pane) = self.panes.get_mut(&resize.pane_id) else {
                    return self.send_no_pane(client_id, resize.pane_id);
                };
                pane.cols = resize.cols.clamp(1, u32::from(u16::MAX)) as u16;
                pane.rows = resize.rows.clamp(1, u32::from(u16::MAX)) as u16;
                let _ = pane.to_pane.send(packet);
                self.broadcast_pane_list();
            }
            Msg::Key(input) => self.forward(client_id, input.pane_id, packet),
            Msg::Mouse(input) => self.forward(client_id, input.pane_id, packet),
            Msg::Paste(input) => self.forward(client_id, input.pane_id, packet),
            Msg::Raw(input) => self.forward(client_id, input.pane_id, packet),
            Msg::Scroll(input) => self.forward(client_id, input.pane_id, packet),
        }
    }

    /// Pass a client packet to the pane process as-is.
    fn forward(&mut self, client_id: ClientId, pane_id: PaneId, packet: Bytes) {
        match self.panes.get(&pane_id) {
            Some(pane) => {
                let _ = pane.to_pane.send(packet);
            }
            None => self.send_no_pane(client_id, pane_id),
        }
    }

    /// Start a pane process for the request. The client hears back once the
    /// process reports that the shell is running, or that it could not start.
    fn create_pane(&mut self, client_id: ClientId, create: &CreatePane) -> Result<()> {
        let cols = create.cols.clamp(1, u32::from(u16::MAX)) as u16;
        let rows = create.rows.clamp(1, u32::from(u16::MAX)) as u16;
        let pane_id = self.next_pane_id;
        let to_pane = start_pane_process(pane_id, create, cols, rows, self.events.clone())?;
        self.next_pane_id += 1;

        let pane = Pane {
            command: create.command.clone(),
            title: String::new(),
            cwd: if create.cwd.is_empty() {
                std::env::current_dir()
                    .map(|dir| dir.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                create.cwd.clone()
            },
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis() as u64),
            cols,
            rows,
            subscribers: HashSet::new(),
            to_pane,
        };
        self.starting.insert(
            pane_id,
            Starting {
                pane,
                client_id,
                request_id: create.request_id,
            },
        );
        Ok(())
    }

    fn on_pane_started(&mut self, pane_id: PaneId) {
        let Some(Starting {
            pane,
            client_id,
            request_id,
        }) = self.starting.remove(&pane_id)
        else {
            return;
        };
        tracing::info!(pane = pane_id, cols = pane.cols, rows = pane.rows, "pane created");
        let info = pane.info(pane_id);
        self.panes.insert(pane_id, pane);
        self.send(
            client_id,
            server_message::Msg::PaneCreated(PaneCreated {
                pane: Some(info),
                request_id,
            }),
        );
        self.broadcast_pane_list();
    }

    fn on_pane_output(&mut self, pane_id: PaneId, packet: Bytes) {
        let Some(pane) = self.panes.get(&pane_id) else {
            return;
        };
        let mut stuck = Vec::new();
        for &client_id in &pane.subscribers {
            if let Some(client) = self.clients.get(&client_id)
                && client.tx.try_send(packet.clone()).is_err()
            {
                stuck.push(client_id);
            }
        }
        // A client that lost a diff has a corrupt screen; disconnecting it is
        // the honest outcome.
        for client_id in stuck {
            tracing::warn!(client = client_id, "client too slow, dropping");
            self.drop_client(client_id);
        }
    }

    fn on_pane_link_closed(
        &mut self,
        pane_id: PaneId,
        exit_code: Option<i32>,
        error: Option<String>,
    ) {
        if let Some(starting) = self.starting.remove(&pane_id) {
            let reason = error.unwrap_or_else(|| "pane process died during startup".into());
            return self.send_error(starting.client_id, &format!("create pane: {reason}"));
        }
        if !self.panes.contains_key(&pane_id) {
            return;
        }
        let exit_code = exit_code.unwrap_or_else(|| {
            tracing::error!(pane = pane_id, "pane process died, taking its shell with it");
            -1
        });

        self.panes.remove(&pane_id);
        tracing::info!(pane = pane_id, exit_code, "pane exited");
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::PaneExited(
            PaneExited { pane_id, exit_code },
        )));
        for client in self.clients.values_mut() {
            client.subscriptions.remove(&pane_id);
            let _ = client.tx.try_send(packet.clone());
        }
        self.broadcast_pane_list();
    }

    fn drop_client(&mut self, client_id: ClientId) {
        let Some(client) = self.clients.remove(&client_id) else {
            return;
        };
        let had_subscriptions = !client.subscriptions.is_empty();
        for pane_id in client.subscriptions {
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.subscribers.remove(&client_id);
            }
        }
        if had_subscriptions {
            self.broadcast_pane_list();
        }
    }

    fn pane_list(&self, request_id: u64) -> PaneList {
        let mut panes: Vec<PaneInfo> = self
            .panes
            .iter()
            .map(|(&id, pane)| pane.info(id))
            .collect();
        panes.sort_by_key(|pane| pane.pane_id);
        PaneList { panes, request_id }
    }

    /// Tell every client what panes exist now, so pickers and tab bars stay
    /// current without polling.
    fn broadcast_pane_list(&mut self) {
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::PaneList(
            self.pane_list(0),
        )));
        let stuck: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, client)| client.tx.try_send(packet.clone()).is_err())
            .map(|(&id, _)| id)
            .collect();
        for client_id in stuck {
            self.drop_client(client_id);
        }
    }

    fn send(&mut self, client_id: ClientId, msg: server_message::Msg) {
        let packet = tilr_proto::encode(&ServerMessage::from(msg));
        let stuck = self
            .clients
            .get(&client_id)
            .is_some_and(|client| client.tx.try_send(packet).is_err());
        if stuck {
            self.drop_client(client_id);
        }
    }

    fn send_error(&mut self, client_id: ClientId, message: &str) {
        self.send(
            client_id,
            server_message::Msg::Error(ProtoError {
                message: message.to_owned(),
            }),
        );
    }

    fn send_no_pane(&mut self, client_id: ClientId, pane_id: PaneId) {
        self.send_error(client_id, &format!("no such pane: {pane_id}"));
    }
}

impl Pane {
    fn info(&self, pane_id: PaneId) -> PaneInfo {
        PaneInfo {
            pane_id,
            cols: u32::from(self.cols),
            rows: u32::from(self.rows),
            command: self.command.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            viewers: self.subscribers.len() as u32,
            created_at_ms: self.created_at_ms,
        }
    }
}

/// Start the pane process, which opens the pty and runs the shell, and wire
/// it to the event loop over its stdin and stdout. Returns the sender for
/// packets going to it.
fn start_pane_process(
    pane_id: PaneId,
    create: &CreatePane,
    cols: u16,
    rows: u16,
    events: mpsc::UnboundedSender<Event>,
) -> Result<mpsc::UnboundedSender<Bytes>> {
    let exe = std::env::current_exe().context("locate own executable")?;
    let mut cmd = Command::new(exe);
    cmd.arg("pane")
        .args(["--id", &pane_id.to_string()])
        .args(["--cols", &cols.to_string()])
        .args(["--rows", &rows.to_string()]);
    if !create.cwd.is_empty() {
        cmd.args(["--cwd", &create.cwd]);
    }
    cmd.arg("--")
        .args(&create.command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().context("spawn pane process")?;
    let mut writer = child.stdin.take().context("pane process stdin")?;
    let mut reader = child.stdout.take().context("pane process stdout")?;

    let (to_pane, mut outgoing) = mpsc::unbounded_channel::<Bytes>();

    tokio::spawn(async move {
        while let Some(packet) = outgoing.recv().await {
            if link::write_packet(&mut writer, &packet).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut exit_code = None;
        let mut error = None;
        while let Ok(Some(mut packet)) = link::read_packet(&mut reader).await {
            if packet.is_empty() {
                continue;
            }
            let kind = packet.split_to(1)[0];
            let event = match kind {
                link::kind::RELAY => Event::PaneOutput { pane_id, packet },
                link::kind::STATUS => match tilr_proto::decode::<PaneStatus>(&packet) {
                    Ok(status) => Event::PaneStatus { pane_id, status },
                    Err(_) => continue,
                },
                link::kind::STARTED => Event::PaneStarted { pane_id },
                link::kind::EXITED => {
                    exit_code = tilr_proto::decode::<PaneExited>(&packet)
                        .ok()
                        .map(|exited| exited.exit_code);
                    continue;
                }
                link::kind::FAILED => {
                    error = tilr_proto::decode::<ProtoError>(&packet)
                        .ok()
                        .map(|failed| failed.message);
                    continue;
                }
                _ => continue,
            };
            if events.send(event).is_err() {
                return;
            }
        }
        let _ = child.wait().await;
        let _ = events.send(Event::PaneLinkClosed {
            pane_id,
            exit_code,
            error,
        });
    });

    Ok(to_pane)
}
