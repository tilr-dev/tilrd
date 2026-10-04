//! Supervisor: runs one pane process per pane and routes packets between
//! WebSocket clients and pane processes. The pty and the shell belong to the
//! pane process, so a pane lives exactly as long as its process.
//!
//! All state lives in one actor task ([`Supervisor::run`]); connection and
//! pane tasks only talk to it through [`Event`]s.
//!
//! The supervisor can replace itself with the executable on disk and keep
//! its panes; see [`crate::reload`].

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, ClipboardAck, ClipboardPut, CreatePane, Error as ProtoError, HelloAck,
    PROTOCOL_VERSION, PaneCreated, PaneExited, PaneInfo, PaneList, PaneState, PaneStatus,
    ServerMessage, Subscribe, SupervisorState, client_message::Msg, server_message,
};
use tokio::io::AsyncReadExt;
use tokio::net::unix::pipe;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::{clipboard, link, pty, reload};

/// Packets queued for one client before it is considered stuck and dropped.
const CLIENT_QUEUE: usize = 1024;
const READ_BUF: usize = 64 * 1024;
/// How long a reload waits for a pane process to take the input already
/// queued for it.
const DETACH_TIMEOUT: Duration = Duration::from_secs(5);
/// A reload asked for while a pane is starting is tried again this much
/// later.
const RELOAD_RETRY: Duration = Duration::from_millis(200);
/// Feature name for the shared clipboard, see `HelloAck.features`.
const FEATURE_CLIPBOARD: &str = "clipboard";

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
    /// Replace the supervisor and every pane process with the executable on
    /// disk.
    Reload,
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
    pid: u32,
    to_pane: mpsc::UnboundedSender<Bytes>,
    link: Link,
}

/// The tasks moving packets to and from one pane process.
struct Link {
    /// Ends with the pipe once the `to_pane` sender is dropped and everything
    /// queued is written; with `None` if the pane process stopped reading.
    writer: JoinHandle<Option<pipe::Sender>>,
    /// Ends with `None` when the pane process is gone, or with what it held
    /// when asked to stop.
    reader: JoinHandle<Option<Detached>>,
    stop_reader: Arc<Notify>,
}

/// What a stopped reader hands back: enough to start it again, here or in
/// the supervisor that takes over.
struct Detached {
    process: pty::Process,
    from_pane: pipe::Receiver,
    /// Bytes read that do not yet make a whole packet.
    unparsed: BytesMut,
}

/// A pane whose process has not yet reported that the shell is running.
/// Clients do not see it until then.
struct Starting {
    pane: Pane,
    /// Who asked for the pane and gets the `PaneCreated` or the error.
    client_id: ClientId,
    request_id: u64,
}

pub struct Options {
    pub listen: SocketAddr,
    pub allow_origins: Vec<String>,
    /// `None` turns the shared clipboard off.
    pub clipboard: Option<clipboard::Limits>,
    /// Reload when the executable on disk changes.
    pub auto_reload: bool,
    /// Descriptor of the state file of the supervisor this one replaces.
    pub resume: Option<RawFd>,
}

pub fn run(options: Options) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let state = options.resume.map(reload::read_state).transpose()?;
            let listener = match &state {
                Some(state) => inherit_listener(state.listener).context("take over listener")?,
                None => TcpListener::bind(options.listen)
                    .await
                    .with_context(|| format!("bind {}", options.listen))?,
            };
            let listener = Arc::new(listener);
            tracing::info!("listening on ws://{}", listener.local_addr()?);

            let (events_tx, events_rx) = mpsc::unbounded_channel();
            tokio::spawn(accept_loop(
                Arc::clone(&listener),
                options.allow_origins.into(),
                events_tx.clone(),
            ));
            tokio::spawn(shutdown_signal(events_tx.clone()));
            tokio::spawn(reload_signal(events_tx.clone()));
            if options.auto_reload {
                let events = events_tx.clone();
                tokio::spawn(reload::watch_executable(crate::exe(), move || {
                    tracing::info!("executable changed on disk");
                    events.send(Event::Reload).is_ok()
                }));
            }

            let (clipboard, pane_env) = match options.clipboard.map(set_up_clipboard) {
                Some(Ok((receiver, env))) => (Some(receiver), env),
                Some(Err(err)) => {
                    tracing::warn!("clipboard is off: {err:#}");
                    (None, Vec::new())
                }
                None => (None, Vec::new()),
            };

            let mut supervisor = Supervisor {
                events: events_tx,
                listener,
                clients: HashMap::new(),
                panes: HashMap::new(),
                starting: HashMap::new(),
                next_pane_id: 1,
                clipboard,
                pane_env,
            };
            if let Some(state) = state {
                supervisor.resume(state);
            }
            supervisor.run(events_rx).await;
            Ok(())
        })
}

/// Take over the listening socket the previous supervisor left open.
fn inherit_listener(fd: RawFd) -> std::io::Result<TcpListener> {
    // SAFETY: the supervisor that exec'd this process left the descriptor
    // open for it and nothing else here owns it.
    let listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
    pty::set_cloexec(fd, true)?;
    listener.set_nonblocking(true)?;
    TcpListener::from_std(listener)
}

/// Prepare the clipboard store and the `xclip` / `wl-paste` links. Returns
/// the receiver for uploads and the environment that points the panes at
/// both.
fn set_up_clipboard(
    limits: clipboard::Limits,
) -> Result<(clipboard::Receiver, Vec<(OsString, OsString)>)> {
    let runtime_dir = clipboard::runtime_dir();
    let shims = clipboard::install_shims(&runtime_dir, crate::exe())?;
    let dir = clipboard::clipboard_dir(&runtime_dir);
    let store = clipboard::Store::create(dir.clone())?;

    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<_> = std::env::split_paths(&path)
        .filter(|dir| *dir != shims)
        .collect();
    dirs.insert(0, shims);
    let path = std::env::join_paths(dirs).context("build PATH for the panes")?;
    let env = vec![
        (OsString::from("PATH"), path),
        (OsString::from(clipboard::DIR_ENV), dir.into_os_string()),
    ];
    Ok((clipboard::Receiver::new(store, limits), env))
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

/// SIGUSR1 reloads the supervisor and the pane processes, to pick up a
/// rebuilt executable without closing any shell.
async fn reload_signal(events: mpsc::UnboundedSender<Event>) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut usr1 = signal(SignalKind::user_defined1()).expect("install SIGUSR1 handler");
    while usr1.recv().await.is_some() {
        if events.send(Event::Reload).is_err() {
            break;
        }
    }
}

async fn accept_loop(
    listener: Arc<TcpListener>,
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
    listener: Arc<TcpListener>,
    clients: HashMap<ClientId, Client>,
    panes: HashMap<PaneId, Pane>,
    starting: HashMap<PaneId, Starting>,
    next_pane_id: PaneId,
    /// `None` when the clipboard is turned off.
    clipboard: Option<clipboard::Receiver>,
    /// Environment added to every pane process, and so to its shell.
    pane_env: Vec<(OsString, OsString)>,
}

impl Supervisor {
    async fn run(mut self, mut events: mpsc::UnboundedReceiver<Event>) {
        while let Some(event) = events.recv().await {
            match event {
                Event::Reload => {
                    if self.reload(&mut events).await {
                        break;
                    }
                }
                Event::Shutdown => break,
                event => self.on_event(event),
            }
        }
        // Returning closes the pipes to the pane processes, which makes them
        // exit; each shell is hung up when its pty closes.
        tracing::info!("shutting down, closing {} pane(s)", self.panes.len());
    }

    fn on_event(&mut self, event: Event) {
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
            // Handled by the event loop.
            Event::Reload | Event::Shutdown => {}
        }
    }

    /// Adopt the panes of the supervisor this one replaced, and have their
    /// processes pick up the new executable too.
    fn resume(&mut self, state: SupervisorState) {
        self.next_pane_id = state.next_pane_id.max(1);
        for pane in state.panes {
            let info = pane.info.unwrap_or_default();
            let pane_id = info.pane_id;
            // SAFETY: the supervisor that exec'd this process left both
            // descriptors open for it and nothing else here owns them.
            let (to_pane, from_pane) = unsafe {
                (
                    OwnedFd::from_raw_fd(pane.to_pane),
                    OwnedFd::from_raw_fd(pane.from_pane),
                )
            };
            let pipes = pty::set_cloexec(to_pane.as_raw_fd(), true)
                .and_then(|()| pty::set_cloexec(from_pane.as_raw_fd(), true))
                .and_then(|()| {
                    Ok((
                        pipe::Sender::from_owned_fd(to_pane)?,
                        pipe::Receiver::from_owned_fd(from_pane)?,
                    ))
                });
            let (to_pane, from_pane) = match pipes {
                Ok(pipes) => pipes,
                Err(err) => {
                    tracing::error!(pane = pane_id, "pane lost in reload: {err}");
                    continue;
                }
            };
            let detached = Detached {
                process: pty::Process::Inherited(pane.pid),
                from_pane,
                unparsed: BytesMut::from(pane.unparsed.as_slice()),
            };
            let (sender, link) = attach(pane_id, to_pane, detached, self.events.clone());
            // An empty packet is the reload request.
            let _ = sender.send(Bytes::new());
            self.panes.insert(
                pane_id,
                Pane {
                    command: info.command,
                    title: info.title,
                    cwd: info.cwd,
                    created_at_ms: info.created_at_ms,
                    cols: info.cols.clamp(1, u32::from(u16::MAX)) as u16,
                    rows: info.rows.clamp(1, u32::from(u16::MAX)) as u16,
                    subscribers: HashSet::new(),
                    pid: pane.pid,
                    to_pane: sender,
                    link,
                },
            );
        }
        tracing::info!("resumed after reload with {} pane(s)", self.panes.len());
    }

    /// Replace this process with the executable now on disk, which carries
    /// on with the same panes. Returns only if that did not happen: `true`
    /// if the supervisor should shut down, `false` if it carries on itself.
    async fn reload(&mut self, events: &mut mpsc::UnboundedReceiver<Event>) -> bool {
        // A pane that is starting has not reported yet; wait for it rather
        // than carry a half-started pane over.
        if !self.starting.is_empty() {
            let events = self.events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(RELOAD_RETRY).await;
                let _ = events.send(Event::Reload);
            });
            return false;
        }
        let exe = crate::exe();
        if let Err(err) = reload::check_executable(exe).await {
            tracing::error!("not reloading: {err:#}");
            return false;
        }
        tracing::info!(exe = %exe.display(), panes = self.panes.len(), "reloading");

        // Stop reading from the pane processes, each between two packets.
        for pane in self.panes.values() {
            pane.link.stop_reader.notify_one();
        }
        let mut detached = HashMap::new();
        for (&pane_id, pane) in &mut self.panes {
            if let Ok(Some(reader)) = (&mut pane.link.reader).await {
                detached.insert(pane_id, reader);
            }
        }
        // Deal with what the readers and the clients sent before this point.
        // It also drops the panes whose process turned out to be gone.
        while let Ok(event) = events.try_recv() {
            match event {
                Event::Shutdown => return true,
                event => self.on_event(event),
            }
        }

        // Let each pane process take the input queued for it, then stop
        // writing. A process that does not take it is stuck and is given up.
        let mut handed_over = Vec::new();
        for (pane_id, reader) in detached {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                continue;
            };
            pane.to_pane = mpsc::unbounded_channel().0;
            let writer = tokio::time::timeout(DETACH_TIMEOUT, &mut pane.link.writer).await;
            match writer {
                Ok(Ok(Some(to_pane))) => handed_over.push((pane_id, to_pane, reader)),
                _ => {
                    tracing::error!(pane = pane_id, "pane process is stuck, closing it");
                    pane.link.writer.abort();
                    // SAFETY: kill has no memory-safety preconditions.
                    unsafe {
                        libc::kill(pane.pid as libc::pid_t, libc::SIGKILL);
                    }
                    let mut process = reader.process;
                    tokio::spawn(async move { process.wait().await });
                    self.on_pane_link_closed(pane_id, None, None);
                }
            }
        }

        let err = self.exec_successor(&handed_over);
        tracing::error!("reload failed: {err:#}");
        // Carry on as before.
        for (pane_id, to_pane, reader) in handed_over {
            let (sender, link) = attach(pane_id, to_pane, reader, self.events.clone());
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.to_pane = sender;
                pane.link = link;
            }
        }
        false
    }

    /// Exec the executable on disk, leaving it the listening socket, the
    /// pipes to the pane processes and a description of the panes. Returns
    /// only on failure, with every descriptor as it was.
    fn exec_successor(&self, panes: &[(PaneId, pipe::Sender, Detached)]) -> anyhow::Error {
        let listener = self.listener.as_raw_fd();
        let mut inherited = vec![listener];
        let state = SupervisorState {
            listener,
            next_pane_id: self.next_pane_id,
            panes: panes
                .iter()
                .filter_map(|(pane_id, to_pane, reader)| {
                    let pane = self.panes.get(pane_id)?;
                    inherited.extend([to_pane.as_raw_fd(), reader.from_pane.as_raw_fd()]);
                    Some(PaneState {
                        info: Some(pane.info(*pane_id)),
                        pid: pane.pid,
                        to_pane: to_pane.as_raw_fd(),
                        from_pane: reader.from_pane.as_raw_fd(),
                        unparsed: reader.unparsed.to_vec(),
                    })
                })
                .collect(),
        };
        let file = match reload::write_state(&state) {
            Ok(file) => file,
            Err(err) => return anyhow::Error::new(err).context("write state file"),
        };
        inherited.push(file.as_raw_fd());

        // These descriptors must survive the exec.
        let set_inheritable = |inheritable: bool| {
            inherited
                .iter()
                .try_for_each(|&fd| pty::set_cloexec(fd, !inheritable))
        };
        if let Err(err) = set_inheritable(true) {
            let _ = set_inheritable(false);
            return anyhow::Error::new(err).context("make descriptors inheritable");
        }
        let exe = crate::exe();
        let err = std::process::Command::new(exe)
            .args(reload::own_args())
            .arg("--resume")
            .arg(file.as_raw_fd().to_string())
            .exec();
        let _ = set_inheritable(false);
        anyhow::Error::new(err).context(format!("exec {}", exe.display()))
    }

    fn on_client_packet(&mut self, client_id: ClientId, packet: Bytes) {
        let msg = match tilr_proto::decode::<ClientMessage>(&packet) {
            Ok(ClientMessage { msg: Some(msg) }) => msg,
            // A message type this server does not know; see tilr.proto.
            Ok(_) => return,
            Err(err) => return self.send_error(client_id, &format!("bad packet: {err}")),
        };
        match msg {
            Msg::Hello(_) => {
                let clipboard_max_bytes =
                    self.clipboard.as_ref().map(clipboard::Receiver::max_bytes);
                self.send(
                    client_id,
                    server_message::Msg::HelloAck(HelloAck {
                        protocol_version: PROTOCOL_VERSION,
                        server_version: env!("CARGO_PKG_VERSION").into(),
                        features: clipboard_max_bytes
                            .map(|_| FEATURE_CLIPBOARD.to_owned())
                            .into_iter()
                            .collect(),
                        clipboard_max_bytes: clipboard_max_bytes.unwrap_or(0),
                    }),
                );
            }
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
            Msg::ClipboardPut(put) => self.on_clipboard_put(client_id, put),
        }
    }

    fn on_clipboard_put(&mut self, client_id: ClientId, put: ClipboardPut) {
        let ack = match &mut self.clipboard {
            Some(clipboard) => clipboard.on_chunk(client_id, put),
            // Refuse once, on the chunk that opens the transfer.
            None => (!put.mime.is_empty()).then(|| ClipboardAck {
                transfer_id: put.transfer_id,
                ok: false,
                error: "the clipboard is turned off on this server".into(),
            }),
        };
        if let Some(ack) = ack {
            if !ack.ok {
                tracing::warn!(
                    client = client_id,
                    "clipboard upload refused: {}",
                    ack.error
                );
            }
            self.send(client_id, server_message::Msg::ClipboardAck(ack));
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
        let (pid, to_pane, link) = start_pane_process(
            pane_id,
            create,
            cols,
            rows,
            &self.pane_env,
            self.events.clone(),
        )?;
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
            pid,
            to_pane,
            link,
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
        tracing::info!(
            pane = pane_id,
            cols = pane.cols,
            rows = pane.rows,
            "pane created"
        );
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
            tracing::error!(
                pane = pane_id,
                "pane process died, taking its shell with it"
            );
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
        if let Some(clipboard) = &mut self.clipboard {
            clipboard.drop_client(client_id);
        }
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
        let mut panes: Vec<PaneInfo> = self.panes.iter().map(|(&id, pane)| pane.info(id)).collect();
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
/// it to the event loop over its stdin and stdout. Returns its pid, the
/// sender for packets going to it, and the tasks behind both directions.
fn start_pane_process(
    pane_id: PaneId,
    create: &CreatePane,
    cols: u16,
    rows: u16,
    env: &[(OsString, OsString)],
    events: mpsc::UnboundedSender<Event>,
) -> Result<(u32, mpsc::UnboundedSender<Bytes>, Link)> {
    let mut cmd = Command::new(crate::exe());
    cmd.arg("pane")
        .args(["--id", &pane_id.to_string()])
        .args(["--cols", &cols.to_string()])
        .args(["--rows", &rows.to_string()]);
    if !create.cwd.is_empty() {
        cmd.args(["--cwd", &create.cwd]);
    }
    cmd.arg("--")
        .args(&create.command)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().context("spawn pane process")?;
    let pid = child.id().context("pane process pid")?;
    // Plain pipes rather than tokio's child handles: on a reload they are
    // handed to the next supervisor as bare descriptors.
    let to_pane = child.stdin.take().context("pane process stdin")?;
    let from_pane = child.stdout.take().context("pane process stdout")?;
    let to_pane =
        pipe::Sender::from_owned_fd(to_pane.into_owned_fd()?).context("pane process stdin")?;
    let from_pane =
        pipe::Receiver::from_owned_fd(from_pane.into_owned_fd()?).context("pane process stdout")?;

    let detached = Detached {
        process: pty::Process::Spawned(child),
        from_pane,
        unparsed: BytesMut::new(),
    };
    let (sender, link) = attach(pane_id, to_pane, detached, events);
    Ok((pid, sender, link))
}

/// Start the tasks that move packets to and from a pane process.
fn attach(
    pane_id: PaneId,
    to_pane: pipe::Sender,
    reader: Detached,
    events: mpsc::UnboundedSender<Event>,
) -> (mpsc::UnboundedSender<Bytes>, Link) {
    let (sender, outgoing) = mpsc::unbounded_channel();
    let stop_reader = Arc::new(Notify::new());
    let link = Link {
        writer: tokio::spawn(write_pane(to_pane, outgoing)),
        reader: tokio::spawn(read_pane(pane_id, reader, Arc::clone(&stop_reader), events)),
        stop_reader,
    };
    (sender, link)
}

/// Write queued packets to the pane process until the queue's sender is
/// dropped, then return the pipe. `None` if the process stopped reading.
async fn write_pane(
    mut to_pane: pipe::Sender,
    mut outgoing: mpsc::UnboundedReceiver<Bytes>,
) -> Option<pipe::Sender> {
    while let Some(packet) = outgoing.recv().await {
        link::write_packet(&mut to_pane, &packet).await.ok()?;
    }
    Some(to_pane)
}

/// Turn what the pane process sends into events, until it exits (`None`) or
/// `stop` is notified, which hands back everything needed to carry on.
async fn read_pane(
    pane_id: PaneId,
    reader: Detached,
    stop: Arc<Notify>,
    events: mpsc::UnboundedSender<Event>,
) -> Option<Detached> {
    let Detached {
        mut process,
        mut from_pane,
        mut unparsed,
    } = reader;
    let mut exit_code = None;
    let mut error = None;
    'read: loop {
        loop {
            let mut packet = match link::take_packet(&mut unparsed) {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(_) => break 'read,
            };
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
                return None;
            }
        }
        unparsed.reserve(READ_BUF);
        tokio::select! {
            biased;
            () = stop.notified() => {
                return Some(Detached { process, from_pane, unparsed });
            }
            // Reading into the buffer is cancel-safe: nothing is lost if the
            // stop wins.
            read = from_pane.read_buf(&mut unparsed) => match read {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
    let _ = process.wait().await;
    let _ = events.send(Event::PaneLinkClosed {
        pane_id,
        exit_code,
        error,
    });
    None
}
