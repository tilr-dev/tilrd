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
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, ClipboardAck, ClipboardPut, CreatePane, Error as ProtoError, HelloAck,
    HostEvent, PROTOCOL_VERSION, PaneCreated, PaneExited, PaneInfo, PaneList, PaneState,
    PaneStatus, Pong, ServerMessage, Subscribe, SupervisorState, client_message::Msg,
    server_message,
};
use tokio::io::AsyncReadExt;
use tokio::net::unix::pipe;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use uuid::Uuid;

use crate::pane::{DEFAULT_COLS, DEFAULT_ROWS};
use crate::{agent, auth, clipboard, cloud, files, host, identity, link, pty, reload, socks};

/// Packets queued for one client before it is considered stuck and dropped.
const CLIENT_QUEUE: usize = 1024;
const READ_BUF: usize = 64 * 1024;
/// How long a reload waits for a pane process to take the input already
/// queued for it.
const DETACH_TIMEOUT: Duration = Duration::from_secs(5);
/// A reload asked for while a pane is starting is tried again this much
/// later.
const RELOAD_RETRY: Duration = Duration::from_millis(200);
/// Pings sent to a client within the time it may stay silent, so that one
/// lost ping does not cost the connection.
const PINGS_PER_TIMEOUT: u32 = 3;
/// Feature name for the shared clipboard, see `HelloAck.features`.
const FEATURE_CLIPBOARD: &str = "clipboard";
/// Feature name for `Ping`, see `HelloAck.features`.
const FEATURE_PING: &str = "ping";
/// Feature name for the files protocol, see `HelloAck.features`.
const FEATURE_FILES: &str = "files";
/// Feature name for `HostEvent`, see `HelloAck.features`.
const FEATURE_HOST_EVENTS: &str = "host-events";
/// Feature name for SSH agent forwarding, see `HelloAck.features`.
const FEATURE_AGENT: &str = "agent";

type ClientId = u64;
/// Made up when the pane is created; on the wire in its text form.
type PaneId = Uuid;

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
    /// The answer to a files request, worked out off the actor.
    Files(files::Output),
    /// What the agent's tasks report, see [`crate::agent`].
    Agent(agent::Output),
    /// A program in a pane asks the clients watching it for something, see
    /// [`crate::host`]. `reply` gets how many clients it went to.
    Host {
        request: host::Request,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    /// Replace the supervisor and every pane process with the executable on
    /// disk.
    Reload,
    Shutdown,
}

struct Client {
    tx: mpsc::Sender<Bytes>,
    subscriptions: HashSet<PaneId>,
    /// Said `Hello` with the protocol version this server speaks. Nothing
    /// else is taken from the client before that.
    greeted: bool,
    /// Announced "host-events" in `Hello.features`: it is sent `HostEvent`s.
    host_events: bool,
    /// Announced "agent" in `Hello.features`: it can serve the SSH agent.
    agent: bool,
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
    /// Ask every client for the access token.
    pub auth: bool,
    /// The `cloudflared` to run for a tunnel.
    pub cloudflared: PathBuf,
    /// `None` turns the shared clipboard off.
    pub clipboard: Option<clipboard::Limits>,
    /// Reload when the executable on disk changes.
    pub auto_reload: bool,
    /// Drop a client nothing was heard from for this long, pings included.
    /// `None` keeps silent clients forever.
    pub client_timeout: Option<Duration>,
    /// Serve the SOCKS proxy, see [`crate::socks`].
    pub socks: bool,
    /// `None` turns the files protocol off, see [`crate::files`].
    pub files: Option<files::Settings>,
    /// Forward the SSH agent of the clients to the panes, see
    /// [`crate::agent`].
    pub agent: bool,
}

/// Who may come in through the listening port, and to what.
struct Door {
    allow_origins: Vec<String>,
    /// The access token; `None` when the server runs without one.
    token: Option<String>,
    /// Whether the SOCKS proxy is served.
    socks: bool,
}

pub fn run(options: Options) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let state = reload::take_state()?;
            reload::sweep_stale();
            if state.is_some() {
                crate::fail::point("supervisor-resume")?;
            }
            let listener = match &state {
                Some(state) => inherit_listener(state.listener).context("take over listener")?,
                None => TcpListener::bind(options.listen)
                    .await
                    .with_context(|| format!("bind {}", options.listen))?,
            };
            let listener = Arc::new(listener);
            let token = if options.auth {
                Some(
                    auth::token()
                        .context("the access token (start with --no-auth to do without)")?,
                )
            } else {
                tracing::warn!("no authentication: anyone who can reach this server gets a shell");
                None
            };
            if options.socks {
                tracing::info!(
                    "SOCKS proxy on: WebSocket {} and raw SOCKS5 on this port",
                    socks::PATH
                );
            } else {
                tracing::info!("SOCKS proxy off");
            }

            let mut local = listener.local_addr()?;
            tracing::info!("listening on ws://{local}");
            if local.ip().is_unspecified() {
                local.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
            }
            let tunnel = cloud::spawn(cloud::Options {
                auth: token.is_some(),
                local,
                cloudflared: options.cloudflared,
            });

            let (events_tx, events_rx) = mpsc::unbounded_channel();
            tokio::spawn(accept_loop(
                Arc::clone(&listener),
                Arc::new(Door {
                    allow_origins: options.allow_origins,
                    token: token.clone(),
                    socks: options.socks,
                }),
                options.client_timeout,
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

            let server_id = identity::server_id();
            let host_socket = set_up_host_socket(&server_id, events_tx.clone());
            let (clipboard, mut pane_env) = set_up_shims(options.clipboard, host_socket.is_some());
            if let Some(path) = &host_socket {
                pane_env.push((
                    OsString::from(host::SOCK_ENV),
                    path.clone().into_os_string(),
                ));
                pane_env.push((
                    OsString::from(host::SERVER_ENV),
                    server_id.to_string().into(),
                ));
            }
            let files = options.files.map(|settings| {
                let events = events_tx.clone();
                files::Files::new(
                    settings,
                    Arc::new(move |output| {
                        let _ = events.send(Event::Files(output));
                    }),
                )
            });
            if files.is_none() {
                tracing::info!("files off");
            }
            let agent = options.agent.then(|| {
                let events = events_tx.clone();
                agent::Agent::new(Arc::new(move |output| {
                    let _ = events.send(Event::Agent(output));
                }))
            });
            if agent.is_some() {
                tracing::info!(target: "tilr_server::agent", "SSH agent forwarding on");
            } else {
                tracing::info!(target: "tilr_server::agent", "SSH agent forwarding off");
            }

            let mut supervisor = Supervisor {
                events: events_tx,
                listener,
                clients: HashMap::new(),
                panes: HashMap::new(),
                starting: HashMap::new(),
                server_id,
                token,
                tunnel,
                clipboard,
                pane_env,
                files,
                host_events: host_socket.is_some(),
                agent,
            };
            if let Some(state) = state {
                supervisor.resume(state);
            }
            supervisor.run(events_rx).await;
            // A reload does not get here: the successor binds the socket anew.
            if let Some(path) = host_socket {
                host::remove(&path);
            }
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

/// Listen for host events (see [`crate::host`]) and return the socket's
/// path. `None`, with a warning, if that cannot be done: the server works on
/// without them.
fn set_up_host_socket(server_id: &Uuid, events: mpsc::UnboundedSender<Event>) -> Option<PathBuf> {
    let path = host::socket_path(&server_id.to_string());
    match host::bind(&path) {
        Ok(listener) => {
            tracing::info!(path = %path.display(), "host events on");
            tokio::spawn(host::serve(
                listener,
                Box::new(move |request, reply| events.send(Event::Host { request, reply }).is_ok()),
            ));
            Some(path)
        }
        Err(err) => {
            tracing::warn!("host events are off: {err:#}");
            None
        }
    }
}

/// Put this executable on the PATH of the panes as the tools they get from
/// the server: `xclip` and `wl-paste` for the clipboard, `tilr` for host
/// events. Returns the clipboard's receiver for uploads, if it is on, and the
/// environment for the panes.
fn set_up_shims(
    clipboard: Option<clipboard::Limits>,
    host_events: bool,
) -> (Option<clipboard::Receiver>, Vec<(OsString, OsString)>) {
    let runtime_dir = clipboard::runtime_dir();
    let mut env = Vec::new();
    let receiver = clipboard.and_then(|limits| {
        let dir = clipboard::clipboard_dir(&runtime_dir);
        match clipboard::Store::create(dir.clone()) {
            Ok(store) => {
                env.push((OsString::from(clipboard::DIR_ENV), dir.into_os_string()));
                Some(clipboard::Receiver::new(store, limits))
            }
            Err(err) => {
                tracing::warn!("clipboard is off: {err:#}");
                None
            }
        }
    });
    let mut names: Vec<&str> = Vec::new();
    if receiver.is_some() {
        names.extend(clipboard::SHIMS);
    }
    if host_events {
        names.push(host::SHIM);
    }
    let all: Vec<&str> = clipboard::SHIMS.into_iter().chain([host::SHIM]).collect();
    let shims = match clipboard::install_shims(&runtime_dir, crate::exe(), &names, &all) {
        Ok(shims) => shims,
        Err(err) => {
            tracing::warn!("no tools for the panes (clipboard, tilr): {err:#}");
            return (None, Vec::new());
        }
    };
    if names.is_empty() {
        return (receiver, env);
    }
    env.push((
        OsString::from(clipboard::SHIMS_ENV),
        shims.clone().into_os_string(),
    ));
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<_> = std::env::split_paths(&path)
        .filter(|dir| *dir != shims)
        .collect();
    dirs.insert(0, shims);
    match std::env::join_paths(dirs) {
        Ok(path) => env.insert(0, (OsString::from("PATH"), path)),
        Err(err) => tracing::warn!("no tools for the panes (clipboard, tilr): PATH: {err}"),
    }
    (receiver, env)
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
    door: Arc<Door>,
    client_timeout: Option<Duration>,
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
                    Arc::clone(&door),
                    client_timeout,
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
    door: Arc<Door>,
    client_timeout: Option<Duration>,
    events: mpsc::UnboundedSender<Event>,
) {
    // Keystrokes and small diffs must not wait for Nagle.
    let _ = stream.set_nodelay(true);
    // An HTTP request starts with a letter; a SOCKS5 client with its version.
    let mut first = [0u8; 1];
    if matches!(stream.peek(&mut first).await, Ok(1)) && first[0] == socks::VERSION {
        if !door.socks {
            tracing::debug!(%addr, "raw SOCKS connection while the proxy is off");
            return;
        }
        let auth = match &door.token {
            Some(token) => socks::Auth::Password {
                token: token.clone(),
            },
            None => socks::Auth::None,
        };
        if let Err(err) = socks::serve(stream, auth, addr).await {
            tracing::debug!(%addr, "SOCKS session: {err:#}");
        }
        return;
    }

    // What the handshake asked for; the callback tells.
    let mut route = Route::Main;
    #[expect(
        clippy::result_large_err,
        reason = "the handshake callback signature is fixed by tungstenite"
    )]
    let admit = |request: &Request, response: Response| {
        let refuse = |status: StatusCode, why: &str| {
            let mut refused = ErrorResponse::new(Some(why.into()));
            *refused.status_mut() = status;
            refused
        };
        let socks = request.uri().path() == socks::PATH;
        if socks && !door.socks {
            return Err(refuse(StatusCode::NOT_FOUND, "not found"));
        }
        // A browser attaches the page's origin to every WebSocket handshake.
        // With no other authentication, this check is what stops an
        // arbitrary website from opening a shell on this machine.
        if let Some(origin) = request.headers().get("origin") {
            let origin = origin.to_str().unwrap_or("");
            if !door.allow_origins.iter().any(|allowed| allowed == origin) {
                tracing::warn!(%addr, origin, "rejected connection from disallowed origin");
                return Err(refuse(StatusCode::FORBIDDEN, "origin not allowed"));
            }
        }
        if socks {
            // The main protocol asks for the token in `Hello`; SOCKS has no
            // such message, so the handshake carries it.
            if let Some(token) = &door.token
                && !socks_token(request).is_some_and(|given| auth::matches(token, given))
            {
                tracing::warn!(%addr, "rejected SOCKS WebSocket without the access token");
                let mut refused = refuse(StatusCode::UNAUTHORIZED, "access token required");
                refused
                    .headers_mut()
                    .insert("www-authenticate", "Bearer".parse().expect("header value"));
                return Err(refused);
            }
            route = Route::Socks;
        }
        Ok(response)
    };
    let ws = match tokio_tungstenite::accept_hdr_async(stream, admit).await {
        Ok(ws) => ws,
        Err(err) => {
            tracing::warn!(%addr, "websocket handshake failed: {err}");
            return;
        }
    };
    if route == Route::Socks {
        tracing::debug!(%addr, "SOCKS WebSocket connected");
        socks::serve_websocket(ws, client_timeout, addr).await;
        return;
    }
    tracing::info!(client = id, %addr, "client connected");
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Bytes>(CLIENT_QUEUE);
    if events.send(Event::ClientConnected { id, tx }).is_err() {
        return;
    }

    // A network that drops out without closing the connection leaves it open
    // for hours. Pings make a client that is still there answer, so one that
    // stays silent is known to be gone.
    let mut ping = client_timeout.map(|timeout| {
        let mut ping = tokio::time::interval(timeout / PINGS_PER_TIMEOUT);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ping
    });
    // Ends when the supervisor drops the client's sender.
    let writer = tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                packet = rx.recv() => match packet {
                    Some(packet) => Message::Binary(packet),
                    None => break,
                },
                () = tick(&mut ping) => Message::Ping(Bytes::new()),
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    loop {
        let message = match client_timeout {
            Some(timeout) => match tokio::time::timeout(timeout, source.next()).await {
                Ok(message) => message,
                Err(_) => {
                    tracing::warn!(client = id, "client went silent, dropping");
                    break;
                }
            },
            None => source.next().await,
        };
        let Some(Ok(message)) = message else {
            break;
        };
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

/// What a WebSocket on the listening port is for.
#[derive(PartialEq)]
enum Route {
    /// The main protocol, whatever the path.
    Main,
    /// The SOCKS proxy, at [`socks::PATH`].
    Socks,
}

/// The access token a SOCKS WebSocket handshake carries: `Authorization:
/// Bearer <token>`, else `?token=<token>` (a page cannot set headers on a
/// WebSocket).
fn socks_token(request: &Request) -> Option<&str> {
    let header = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    header.or_else(|| {
        request
            .uri()
            .query()?
            .split('&')
            .find_map(|pair| pair.strip_prefix("token="))
    })
}

/// The next tick of `interval`; never, without one.
async fn tick(interval: &mut Option<Interval>) {
    match interval {
        Some(interval) => {
            interval.tick().await;
        }
        None => std::future::pending().await,
    }
}

struct Supervisor {
    events: mpsc::UnboundedSender<Event>,
    listener: Arc<TcpListener>,
    clients: HashMap<ClientId, Client>,
    panes: HashMap<PaneId, Pane>,
    starting: HashMap<PaneId, Starting>,
    server_id: Uuid,
    /// What a client must present in `Hello`. `None`: nothing.
    token: Option<String>,
    /// The tunnel of a server linked to tilr Cloud.
    tunnel: cloud::Tunnel,
    /// `None` when the clipboard is turned off.
    clipboard: Option<clipboard::Receiver>,
    /// Environment added to every pane process, and so to its shell.
    pane_env: Vec<(OsString, OsString)>,
    /// `None` when the files protocol is turned off.
    files: Option<files::Files>,
    /// Whether programs in the panes can reach the clients, see
    /// [`crate::host`].
    host_events: bool,
    /// `None` when SSH agent forwarding is turned off.
    agent: Option<agent::Agent>,
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
        if let Some(agent) = &mut self.agent {
            agent.remove_all();
        }
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::ClientConnected { id, tx } => {
                self.clients.insert(
                    id,
                    Client {
                        tx,
                        subscriptions: HashSet::new(),
                        greeted: false,
                        host_events: false,
                        agent: false,
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
            Event::Files(output) => self.on_files_output(output),
            Event::Agent(output) => self.on_agent_output(output),
            Event::Host { request, reply } => {
                let _ = reply.send(self.on_host_event(request));
            }
            // Handled by the event loop.
            Event::Reload | Event::Shutdown => {}
        }
    }

    /// Adopt the panes of the supervisor this one replaced, and have their
    /// processes pick up the new executable too.
    fn resume(&mut self, state: SupervisorState) {
        for pane in state.panes {
            let info = pane.info.unwrap_or_default();
            let Ok(pane_id) = info.pane_id.parse::<PaneId>() else {
                tracing::error!(pane = info.pane_id, "pane lost in reload: bad id");
                continue;
            };
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
                    tracing::error!(pane = %pane_id, "pane lost in reload: {err}");
                    continue;
                }
            };
            let detached = Detached {
                process: pty::Process::Inherited(pane.pid),
                from_pane,
                unparsed: BytesMut::from(pane.unparsed.as_slice()),
            };
            // The shell has the socket's path from before; it is made again
            // at the same place.
            if let Some(agent) = &mut self.agent {
                agent.listen(pane_id);
            }
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
                    tracing::error!(pane = %pane_id, "pane process is stuck, closing it");
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

        // It is not carried over: the successor starts its own.
        self.tunnel.stop();
        let err = match crate::fail::point("reload-before-exec") {
            Ok(()) => self.exec_successor(&handed_over),
            Err(err) => err,
        };
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
            format: reload::STATE_FORMAT,
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
            Err(err) => return err.context("write state file"),
        };

        // These descriptors must survive the exec.
        let set_inheritable = |inheritable: bool| {
            inherited
                .iter()
                .try_for_each(|&fd| pty::set_cloexec(fd, !inheritable))
        };
        let exe = crate::exe();
        let err = match set_inheritable(true) {
            Ok(()) => {
                let err = std::process::Command::new(exe)
                    .args(std::env::args_os().skip(1))
                    .exec();
                anyhow::Error::new(err).context(format!("exec {}", exe.display()))
            }
            Err(err) => anyhow::Error::new(err).context("make descriptors inheritable"),
        };
        let _ = set_inheritable(false);
        let _ = std::fs::remove_file(file);
        err
    }

    fn on_client_packet(&mut self, client_id: ClientId, packet: Bytes) {
        // A client the supervisor has dropped may still have packets on the
        // way. Acting on them would subscribe a client nobody will ever
        // remove again.
        let Some(greeted) = self.clients.get(&client_id).map(|client| client.greeted) else {
            return;
        };
        let msg = match tilr_proto::decode::<ClientMessage>(&packet) {
            Ok(ClientMessage { msg: Some(msg) }) => msg,
            // A message type this server does not know; see tilr.proto.
            Ok(_) => return,
            Err(err) => return self.send_error(client_id, &format!("bad packet: {err}")),
        };
        // A client speaking another version of the protocol would be
        // misunderstood in ways nobody notices; it is turned away instead.
        match &msg {
            Msg::Hello(hello) if hello.protocol_version != PROTOCOL_VERSION => {
                return self.refuse(
                    client_id,
                    &format!(
                        "unsupported protocol version {}: this server speaks {PROTOCOL_VERSION}",
                        hello.protocol_version
                    ),
                );
            }
            Msg::Hello(hello) => match &self.token {
                Some(_) if hello.token.is_empty() => {
                    return self.refuse(client_id, "this server asks for its access token");
                }
                Some(token) if !auth::matches(token, &hello.token) => {
                    return self.refuse(client_id, "wrong access token");
                }
                _ => {}
            },
            _ if !greeted => return self.refuse(client_id, "expected Hello first"),
            _ => {}
        }
        match msg {
            Msg::Hello(hello) => {
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.greeted = true;
                    client.host_events = hello.features.iter().any(|f| f == FEATURE_HOST_EVENTS);
                    client.agent = hello.features.iter().any(|f| f == FEATURE_AGENT);
                }
                let clipboard_max_bytes =
                    self.clipboard.as_ref().map(clipboard::Receiver::max_bytes);
                let fs_max_bytes = self.files.as_ref().map(files::Files::max_bytes);
                let features = [
                    (FEATURE_CLIPBOARD, clipboard_max_bytes.is_some()),
                    (FEATURE_PING, true),
                    (FEATURE_FILES, fs_max_bytes.is_some()),
                    (FEATURE_HOST_EVENTS, self.host_events),
                    (FEATURE_AGENT, self.agent.is_some()),
                ];
                self.send(
                    client_id,
                    server_message::Msg::HelloAck(HelloAck {
                        protocol_version: PROTOCOL_VERSION,
                        server_version: crate::VERSION.into(),
                        features: features
                            .into_iter()
                            .filter(|(_, on)| *on)
                            .map(|(name, _)| name.to_owned())
                            .collect(),
                        clipboard_max_bytes: clipboard_max_bytes.unwrap_or(0),
                        server_id: self.server_id.to_string(),
                        fs_max_bytes: fs_max_bytes.unwrap_or(0),
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
            Msg::ClosePane(close) => self.forward(client_id, &close.pane_id, packet),
            Msg::Subscribe(Subscribe { pane_id: named }) => {
                let Some((pane_id, pane)) = self.pane_mut(&named) else {
                    return self.send_no_pane(client_id, &named);
                };
                pane.subscribers.insert(client_id);
                // The pane answers with a full frame.
                let _ = pane.to_pane.send(packet);
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.subscriptions.insert(pane_id);
                }
                if let Some(agent) = &mut self.agent {
                    agent.subscribed(pane_id, client_id);
                }
                self.broadcast_pane_list();
            }
            Msg::Unsubscribe(unsubscribe) => {
                if let Some((pane_id, pane)) = self.pane_mut(&unsubscribe.pane_id) {
                    pane.subscribers.remove(&client_id);
                    if let Some(client) = self.clients.get_mut(&client_id) {
                        client.subscriptions.remove(&pane_id);
                    }
                    if let Some(agent) = &mut self.agent {
                        agent.unsubscribed(pane_id, client_id);
                    }
                }
                self.broadcast_pane_list();
            }
            Msg::Resize(resize) => {
                let Some((_, pane)) = self.pane_mut(&resize.pane_id) else {
                    return self.send_no_pane(client_id, &resize.pane_id);
                };
                pane.cols = resize.cols.clamp(1, u32::from(u16::MAX)) as u16;
                pane.rows = resize.rows.clamp(1, u32::from(u16::MAX)) as u16;
                let _ = pane.to_pane.send(packet);
                self.broadcast_pane_list();
            }
            Msg::Key(input) => self.forward(client_id, &input.pane_id, packet),
            Msg::Mouse(input) => self.forward(client_id, &input.pane_id, packet),
            Msg::Paste(input) => self.forward(client_id, &input.pane_id, packet),
            Msg::Raw(input) => self.forward(client_id, &input.pane_id, packet),
            Msg::Scroll(input) => self.forward(client_id, &input.pane_id, packet),
            Msg::ClipboardPut(put) => self.on_clipboard_put(client_id, put),
            Msg::Ping(ping) => {
                self.send(
                    client_id,
                    server_message::Msg::Pong(Pong { nonce: ping.nonce }),
                );
            }
            msg @ (Msg::FsList(_)
            | Msg::FsStat(_)
            | Msg::FsRead(_)
            | Msg::FsWrite(_)
            | Msg::FsMkdir(_)
            | Msg::FsRename(_)
            | Msg::FsDelete(_)
            | Msg::FsWatch(_)
            | Msg::FsUnwatch(_)) => match &mut self.files {
                Some(files) => files.on_message(client_id, msg),
                // Not offered, so not asked for by a client that listens.
                None => tracing::debug!(client = client_id, "files request while files are off"),
            },
            Msg::StreamData(data) => match &mut self.agent {
                Some(agent) => {
                    if let Some(reply) = agent.on_data(client_id, data) {
                        self.send(client_id, reply);
                    }
                }
                None => tracing::debug!(client = client_id, "stream data while the agent is off"),
            },
            Msg::StreamClose(close) => match &mut self.agent {
                Some(agent) => agent.on_close(client_id, close),
                None => tracing::debug!(client = client_id, "stream close while the agent is off"),
            },
        }
    }

    fn on_agent_output(&mut self, output: agent::Output) {
        let Some(agent) = &mut self.agent else {
            return;
        };
        match output {
            agent::Output::Connected { pane_id, stream } => {
                let clients = &self.clients;
                agent.connected(pane_id, stream, |client_id| {
                    clients
                        .get(&client_id)
                        .filter(|client| client.agent)
                        .map(|client| client.tx.clone())
                });
            }
            agent::Output::Ended { stream_id } => agent.ended(stream_id),
        }
    }

    fn on_files_output(&mut self, output: files::Output) {
        match output {
            files::Output::Send(client_id, msg) => self.send(client_id, msg),
            files::Output::Watching {
                client,
                request_id,
                path,
                watch,
            } => {
                // A client that left meanwhile: the watch is dropped here.
                if !self.clients.contains_key(&client) {
                    return;
                }
                if let Some(files) = &mut self.files {
                    let ack = files.add_watch(client, request_id, path, watch);
                    self.send(client, ack);
                }
            }
        }
    }

    /// Pass a program's request on to the clients watching its pane that
    /// announced "host-events" in their Hello. Returns how many those are.
    fn on_host_event(&mut self, request: host::Request) -> Result<usize, String> {
        let Some((pane_id, pane)) = self.pane_mut(&request.pane_id) else {
            return Err(format!("no such pane: {}", request.pane_id));
        };
        let subscribers: Vec<ClientId> = pane.subscribers.iter().copied().collect();
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::HostEvent(
            HostEvent {
                pane_id: pane_id.to_string(),
                name: request.name,
                kind: request.kind,
                path: request.path,
                args: request.args,
            },
        )));
        let mut delivered = 0;
        let mut stuck = Vec::new();
        for client_id in subscribers {
            let Some(client) = self.clients.get(&client_id) else {
                continue;
            };
            if !client.host_events {
                continue;
            }
            if client.tx.try_send(packet.clone()).is_ok() {
                delivered += 1;
            } else {
                stuck.push(client_id);
            }
        }
        for client_id in stuck {
            self.drop_client(client_id);
        }
        if delivered == 0 {
            return Err("no tilr client attached".into());
        }
        tracing::info!(pane = %pane_id, clients = delivered, "host event delivered");
        Ok(delivered)
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

    /// The pane a client named, with its id.
    fn pane_mut(&mut self, named: &str) -> Option<(PaneId, &mut Pane)> {
        let pane_id = named.parse().ok()?;
        Some((pane_id, self.panes.get_mut(&pane_id)?))
    }

    /// Pass a client packet to the pane process as-is.
    fn forward(&mut self, client_id: ClientId, named: &str, packet: Bytes) {
        match self.pane_mut(named) {
            Some((_, pane)) => {
                let _ = pane.to_pane.send(packet);
            }
            None => self.send_no_pane(client_id, named),
        }
    }

    /// Start a pane process for the request. The client hears back once the
    /// process reports that the shell is running, or that it could not start.
    fn create_pane(&mut self, client_id: ClientId, create: &CreatePane) -> Result<()> {
        let cols = create.cols.clamp(1, u32::from(u16::MAX)) as u16;
        let rows = create.rows.clamp(1, u32::from(u16::MAX)) as u16;
        let pane_id = PaneId::new_v4();
        let cwd = start_dir(&create.cwd);
        let mut env = self.pane_env.clone();
        if let Some(path) = self.agent.as_mut().and_then(|agent| agent.listen(pane_id)) {
            env.push((OsString::from(agent::SOCK_ENV), path.into_os_string()));
        }
        let started =
            start_pane_process(pane_id, create, &cwd, cols, rows, &env, self.events.clone());
        let (pid, to_pane, link) = match started {
            Ok(started) => started,
            Err(err) => {
                if let Some(agent) = &mut self.agent {
                    agent.close_pane(pane_id);
                }
                return Err(err);
            }
        };

        let pane = Pane {
            command: create.command.clone(),
            title: String::new(),
            cwd,
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
            pane = %pane_id,
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
        if let Some(agent) = &mut self.agent {
            agent.close_pane(pane_id);
        }
        if let Some(starting) = self.starting.remove(&pane_id) {
            let reason = error.unwrap_or_else(|| "pane process died during startup".into());
            return self.send_error(starting.client_id, &format!("create pane: {reason}"));
        }
        if !self.panes.contains_key(&pane_id) {
            return;
        }
        let exit_code = exit_code.unwrap_or_else(|| {
            tracing::error!(
                pane = %pane_id,
                "pane process died, taking its shell with it"
            );
            -1
        });

        self.panes.remove(&pane_id);
        tracing::info!(pane = %pane_id, exit_code, "pane exited");
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::PaneExited(
            PaneExited {
                pane_id: pane_id.to_string(),
                exit_code,
            },
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
        if let Some(files) = &mut self.files {
            files.drop_client(client_id);
        }
        if let Some(agent) = &mut self.agent {
            agent.drop_client(client_id);
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
        // Oldest first; the id only settles panes made in the same millisecond.
        panes.sort_by(|a, b| (a.created_at_ms, &a.pane_id).cmp(&(b.created_at_ms, &b.pane_id)));
        PaneList { panes, request_id }
    }

    /// Tell every client what panes exist now, so pickers and tab bars stay
    /// current without polling. A client that has not said hello yet is left
    /// out: the first thing it hears is the answer to its hello.
    fn broadcast_pane_list(&mut self) {
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::PaneList(
            self.pane_list(0),
        )));
        let stuck: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, client)| client.greeted && client.tx.try_send(packet.clone()).is_err())
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

    /// Tell the client why and close its connection.
    fn refuse(&mut self, client_id: ClientId, reason: &str) {
        tracing::warn!(client = client_id, "refusing client: {reason}");
        self.send_error(client_id, reason);
        // The writer sends what is queued, then closes.
        self.drop_client(client_id);
    }

    fn send_no_pane(&mut self, client_id: ClientId, named: &str) {
        self.send_error(client_id, &format!("no such pane: {named}"));
    }
}

impl Pane {
    fn info(&self, pane_id: PaneId) -> PaneInfo {
        PaneInfo {
            pane_id: pane_id.to_string(),
            cols: u32::from(self.cols),
            rows: u32::from(self.rows),
            command: self.command.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            viewers: self.subscribers.len() as u32,
            created_at_ms: self.created_at_ms,
            pid: self.pid,
        }
    }
}

/// The directory a new pane starts in: the one the client asked for, or the
/// home directory when it asked for none or for one that is not on this
/// machine (a client on another computer may send a path of its own).
fn start_dir(requested: &str) -> String {
    if !requested.is_empty() && std::path::Path::new(requested).is_dir() {
        return requested.to_owned();
    }
    std::env::home_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Start the pane process, which opens the pty and runs the shell, and wire
/// it to the event loop over its stdin and stdout. Returns its pid, the
/// sender for packets going to it, and the tasks behind both directions.
fn start_pane_process(
    pane_id: PaneId,
    create: &CreatePane,
    cwd: &str,
    cols: u16,
    rows: u16,
    env: &[(OsString, OsString)],
    events: mpsc::UnboundedSender<Event>,
) -> Result<(u32, mpsc::UnboundedSender<Bytes>, Link)> {
    let mut cmd = Command::new(crate::exe());
    // What `ps` shows of a pane for as long as it lives: a reload execs the
    // pane process with these same arguments.
    cmd.arg("pane").args(["--id", &pane_id.to_string()]);
    if cols != DEFAULT_COLS {
        cmd.args(["--cols", &cols.to_string()]);
    }
    if rows != DEFAULT_ROWS {
        cmd.args(["--rows", &rows.to_string()]);
    }
    if !cwd.is_empty() {
        cmd.args(["--cwd", cwd]);
    }
    if !create.command.is_empty() {
        cmd.arg("--").args(&create.command);
    }
    cmd.envs(env.iter().map(|(name, value)| (name, value)))
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
