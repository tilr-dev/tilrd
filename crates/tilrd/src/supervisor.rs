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
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, ClipboardAck, ClipboardPut, CreatePane, Error as ProtoError, Frame, HelloAck,
    HostEvent, PROTOCOL_VERSION, PaneCreated, PaneExited, PaneInfo, PaneList, PaneState,
    PaneStatus, Pong, ServerMessage, Subscribe, SupervisorState, Viewer, client_message::Msg,
    close, server_message,
};
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::unix::pipe;
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::process::Command;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Interval, MissedTickBehavior};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use uuid::Uuid;

use crate::link::FrameHead;
use crate::metrics::{self, ClientDrop};
use crate::outbox::{self, Outgoing, ToClient};
use crate::pane::{DEFAULT_COLS, DEFAULT_ROWS};
use crate::{
    agent, auth, clipboard, cloud, exec, files, host, identity, link, local, pty, reload, shell,
    socks,
};

const READ_BUF: usize = 64 * 1024;
/// How long a reload waits for a pane process to take the input already
/// queued for it.
const DETACH_TIMEOUT: Duration = Duration::from_secs(5);
/// A reload asked for while a pane is starting is tried again this much
/// later.
const RELOAD_RETRY: Duration = Duration::from_millis(200);
/// How long a stopping server waits for its pane processes, which take up
/// to 2 s to end a program that ignores SIGHUP.
const PANE_STOP: Duration = Duration::from_secs(3);
/// Pings sent to a client within the time it may stay silent, so that one
/// lost ping does not cost the connection.
const PINGS_PER_TIMEOUT: u32 = 3;
/// How long closing a connection may take: the close frame going out, and
/// the client's answer coming back. A connection that does not close in time
/// is reset, and whatever it still held for the client is thrown away.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
/// How long a connection may take from accept to an open WebSocket (or a
/// SOCKS or metrics request): one that connects and says nothing, or sends
/// its request a byte at a time, is closed then, and holds no descriptor
/// longer.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a main-protocol connection may take from its open WebSocket to
/// its `Hello`, whatever the client timeout (which may be off): one that
/// never greets is closed then (checked on every [`LAG_TICK`]).
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Unsent bytes the kernel holds for a client before the writer waits
/// (`TCP_NOTSENT_LOWAT`): what a slow client is behind on stays in its queue,
/// where frames can still be skipped, instead of in the socket. On a lossy
/// network TCP sends a segment or two per round trip, so every few KB held
/// there are seconds of delay; a fast network takes it as soon as it is
/// written.
const UNSENT_LIMIT: libc::c_int = 4 * 1024;
/// Feature name for the shared clipboard, see `HelloAck.features`.
const FEATURE_CLIPBOARD: &str = "clipboard";
/// Feature name for the files protocol, see `HelloAck.features`.
const FEATURE_FILES: &str = "files";
/// Feature name for `HostEvent`, see `HelloAck.features`.
const FEATURE_HOST_EVENTS: &str = "host-events";
/// Feature name for SSH agent forwarding, see `HelloAck.features`.
const FEATURE_AGENT: &str = "agent";
/// Feature name for Exec, see `HelloAck.features`.
const FEATURE_EXEC: &str = "exec";
/// How long a reload waits for what its clients were last sent (the ends of
/// their execs) to be written out before the connections go.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);
/// How often the event loop's lag is measured (`tilr_supervisor_loop_lag_seconds`).
const LAG_TICK: Duration = Duration::from_secs(1);
/// How long a scrape of `/metrics` waits for the event loop.
const METRICS_TIMEOUT: Duration = Duration::from_secs(5);

type ClientId = u64;
/// Made up when the pane is created; on the wire in its text form.
type PaneId = Uuid;
/// A packet for a pane process; with a client's input, when the supervisor
/// read it from the client (`tilr_route_input_seconds`).
type ToPane = (Bytes, Option<Instant>);

enum Event {
    ClientConnected {
        id: ClientId,
        addr: Peer,
        /// Came in on the Unix socket: this user's, no token asked.
        local: bool,
        tx: ToClient,
        /// Ends the connection.
        close: mpsc::UnboundedSender<Closing>,
    },
    ClientPacket {
        id: ClientId,
        packet: Bytes,
        /// When the connection's task read it.
        received: Instant,
    },
    /// The connection ended: closed by the client or the network, or by its
    /// task for `why`.
    ClientGone {
        id: ClientId,
        why: Option<ClientDrop>,
    },
    /// What was queued for the client is written out, as it asked
    /// ([`ToClient::want_drained`]).
    ClientDrained {
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
        /// When the pane's reader took it off the pipe.
        read_at: Instant,
        /// What the packet is, if the pane process said (a frame).
        head: Option<FrameHead>,
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
    /// What the tasks of execs report, see [`crate::exec`].
    Exec(exec::Output),
    /// A program in a pane asks the clients watching it for something, see
    /// [`crate::host`]. `reply` gets how many clients it went to.
    Host {
        request: host::Request,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    /// The once-a-second tick that measures how late events are handled;
    /// it was due at this instant.
    Tick(Instant),
    /// A scrape of `/metrics` asks for the counters.
    Metrics {
        reply: oneshot::Sender<metrics::Snapshot>,
    },
    /// Replace the supervisor and every pane process with the executable on
    /// disk.
    Reload,
    Shutdown,
}

/// Where a connection comes from.
#[derive(Clone, Copy)]
enum Peer {
    Tcp(SocketAddr),
    /// The Unix socket; the client's pid, if the system says.
    Unix(Option<i32>),
}

impl std::fmt::Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "{addr}"),
            Self::Unix(Some(pid)) => write!(f, "unix (pid {pid})"),
            Self::Unix(None) => write!(f, "unix"),
        }
    }
}

struct Client {
    tx: ToClient,
    close: mpsc::UnboundedSender<Closing>,
    addr: Peer,
    /// Came in on the Unix socket.
    local: bool,
    subscriptions: HashSet<PaneId>,
    /// When its WebSocket opened: one that has not said `Hello`
    /// [`HELLO_TIMEOUT`] later is closed.
    connected: Instant,
    /// Said `Hello` with the protocol version this server speaks. Nothing
    /// else is taken from the client before that.
    greeted: bool,
    /// Announced "host-events" in `Hello.features`: it is sent `HostEvent`s.
    host_events: bool,
    /// Announced "agent" in `Hello.features`: it can serve the SSH agent.
    agent: bool,
    /// `Hello.client_id`, `client_name` and `token`.
    client_id: String,
    name: String,
    token: String,
    /// Panes whose frames this client is skipping until a full one, because
    /// it lagged; see [`Supervisor::on_pane_output`].
    lagging: HashMap<PaneId, Lag>,
}

/// What a client missed of a pane while its frames were skipped.
#[derive(Default)]
struct Lag {
    /// The latest of the client's own input seqs among the skipped frames'
    /// `last_input_seq`, 0 if none: the full frame that ends the lag carries
    /// it, so every input still gets its echo.
    own_seq: u64,
}

/// How a connection is to end, as its writer is told.
enum Closing {
    /// Write out what is queued (a refusal), then close.
    Flush,
    /// Close at once, whatever is queued, with this code and reason.
    Now(u16, &'static str),
}

/// Why the supervisor forgets a client.
#[derive(Clone, Copy)]
enum Gone {
    /// Its connection ended.
    Disconnected,
    /// Turned away (see [`Supervisor::refuse`]).
    Refused,
    /// Said no `Hello` within [`HELLO_TIMEOUT`] of its WebSocket opening.
    NoHello,
    Dropped(ClientDrop),
}

struct Pane {
    command: Vec<String>,
    title: String,
    cwd: String,
    created_at_ms: u64,
    /// When a client last subscribed or sent input (`PaneInfo.used_at_ms`).
    used_at_ms: u64,
    cols: u16,
    rows: u16,
    subscribers: HashSet<ClientId>,
    pid: u32,
    to_pane: mpsc::UnboundedSender<ToPane>,
    link: Link,
    /// What `/metrics` reports for the pane.
    counters: metrics::PaneCounters,
    /// A full frame is asked for, for clients that lagged, and has not come
    /// yet.
    full_asked: bool,
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
    /// The pipes to and from the pane process, as a successor would be
    /// handed them.
    fds: (RawFd, RawFd),
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
    /// The TCP port; `None`: the Unix socket only.
    pub listen: Option<ListenOptions>,
    /// The Unix socket for local clients; `None`: none.
    pub local: Option<LocalOptions>,
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
    /// Let clients run programs, see [`crate::exec`].
    pub exec: bool,
}

pub struct ListenOptions {
    pub addr: SocketAddr,
    /// A port that cannot be had stops the server; else it serves the Unix
    /// socket alone.
    pub required: bool,
}

pub struct LocalOptions {
    pub path: PathBuf,
    /// A socket that cannot be taken stops the server; else it serves TCP
    /// alone.
    pub required: bool,
    /// The lock file's descriptor, locked by the `tilr` that started the
    /// server.
    pub lock_fd: Option<RawFd>,
}

/// Who may come in through the listening port, and to what.
struct Door {
    allow_origins: Vec<String>,
    /// The host the server was told to listen on, as a `Host` header may
    /// name it.
    listen_host: String,
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
            files::sweep();
            if state.is_some() {
                crate::fail::point("supervisor-resume-live")?;
            } else {
                agent::sweep();
            }
            // A reload keeps what the previous image had: its TCP socket, or
            // none.
            let listener = match &state {
                Some(state) if state.listener >= 0 => {
                    Some(inherit_listener(state.listener).context("take over listener")?)
                }
                Some(_) => None,
                None => match &options.listen {
                    Some(listen) => bind_listener(listen).await?,
                    None => None,
                },
            };
            let listener = listener.map(Arc::new);
            let local = match state.as_ref().and_then(handed_over_socket) {
                Some((fd, lock, path)) => {
                    Some(local::inherit(fd, lock, path).context("take over the Unix socket")?)
                }
                None => match options.local {
                    Some(options) => set_up_local(options)?,
                    None => None,
                },
            };
            if listener.is_none() && local.is_none() {
                anyhow::bail!("nothing to listen on");
            }
            if state.is_none() {
                identity::make_state_dir_private()?;
            }
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

            let tunnel = match &listener {
                Some(listener) => {
                    let mut addr = listener.local_addr()?;
                    tracing::info!("listening on ws://{addr}");
                    if addr.ip().is_unspecified() {
                        addr.set_ip(std::net::Ipv4Addr::LOCALHOST.into());
                    }
                    cloud::spawn(cloud::Options {
                        auth: token.is_some(),
                        local: addr,
                        cloudflared: options.cloudflared,
                    })
                }
                None => {
                    tracing::info!("no TCP port: no tilr Cloud tunnel");
                    cloud::Tunnel::off()
                }
            };
            if let Some(local) = &local {
                tracing::info!(path = %local.path().display(), "Unix socket for local clients");
            }

            let (events_tx, events_rx) = mpsc::unbounded_channel();
            let shared_metrics = Arc::new(metrics::Shared::default());
            // Client ids go on from where the server before a reload left
            // them: they make `HelloAck.seq_base`.
            let next_client_id = Arc::new(AtomicU64::new(
                state
                    .as_ref()
                    .map_or(0, |state| state.next_client_id)
                    .max(1),
            ));
            let door = Arc::new(Door {
                allow_origins: options.allow_origins,
                // 0.0.0.0 or :: is not a host a request names.
                listen_host: options
                    .listen
                    .map(|listen| listen.addr.ip())
                    .filter(|ip| !ip.is_unspecified())
                    .map(|ip| ip.to_string())
                    .unwrap_or_default(),
                token: token.clone(),
                socks: options.socks,
            });
            if let Some(listener) = &listener {
                tokio::spawn(accept_loop(
                    Arc::clone(listener),
                    Arc::clone(&door),
                    options.client_timeout,
                    Arc::clone(&shared_metrics),
                    Arc::clone(&next_client_id),
                    events_tx.clone(),
                ));
            }
            if let Some(local) = &local {
                tokio::spawn(accept_local(
                    Arc::clone(&local.listener),
                    door,
                    options.client_timeout,
                    Arc::clone(&shared_metrics),
                    Arc::clone(&next_client_id),
                    events_tx.clone(),
                ));
            }
            tokio::spawn(lag_ticks(events_tx.clone()));
            // The handlers are in place before anything else can happen;
            // then come the signals held across the exec that started this
            // process, if one did.
            crate::fail::point("supervisor-before-signals")?;
            use tokio::signal::unix::{SignalKind, signal};
            let term = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
            let int = signal(SignalKind::interrupt()).context("install SIGINT handler")?;
            let usr1 = signal(SignalKind::user_defined1()).context("install SIGUSR1 handler")?;
            let hup = signal(SignalKind::hangup()).context("install SIGHUP handler")?;
            tokio::spawn(shutdown_signal(term, int, events_tx.clone()));
            tokio::spawn(reload_signal(usr1, events_tx.clone()));
            tokio::spawn(hangup_signal(hup));
            let held = reload::take_held_record();
            let during = if state.is_some() {
                "during the reload"
            } else {
                "while the server started"
            };
            if held.hangup {
                tracing::info!("SIGHUP ignored: the server keeps running without its terminal");
            }
            if held.reload {
                if state.is_some() {
                    tracing::info!("SIGUSR1 came during the reload: reloading again");
                } else {
                    tracing::info!("SIGUSR1 came while the server started: reloading");
                }
                let _ = events_tx.send(Event::Reload);
            }
            for (came, name) in [(held.terminate, "SIGTERM"), (held.interrupt, "SIGINT")] {
                if came {
                    tracing::info!("{name} came {during}");
                }
            }
            if held.terminate || held.interrupt {
                let _ = events_tx.send(Event::Shutdown);
            }
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
            // `tilr` in a pane talks to this server.
            if let Some(local) = &local {
                pane_env.push((
                    OsString::from(tilr_proto::local::SOCKET_ENV),
                    local.path().as_os_str().to_owned(),
                ));
            }
            if let Some(bound) = &host_socket {
                pane_env.push((
                    OsString::from(host::SOCK_ENV),
                    bound.path.clone().into_os_string(),
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
                tracing::info!(target: "tilrd::agent", "SSH agent forwarding on");
            } else {
                tracing::info!(target: "tilrd::agent", "SSH agent forwarding off");
            }
            // Server commands (`tilr token`) run as execs even with exec off.
            let execs = {
                let events = events_tx.clone();
                Some(exec::Execs::new(Arc::new(move |output| {
                    let _ = events.send(Event::Exec(output));
                })))
            };
            if !options.exec {
                tracing::info!(target: "tilrd::exec", "exec off");
            }

            let mut supervisor = Supervisor {
                events: events_tx,
                listener,
                local,
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
                execs,
                exec: options.exec,
                metrics: shared_metrics,
                next_client_id,
            };
            if let Some(state) = state {
                supervisor.resume(state);
            }
            crate::notify::ready();
            let local = supervisor.run(events_rx).await;
            // A reload does not get here: the successor binds the socket anew.
            if let Some(bound) = host_socket {
                bound.remove();
            }
            if let Some(local) = local {
                local.remove();
            }
            Ok(())
        })
}

/// The check child of a reload: show that a supervisor started with
/// `options` resumes from the state in the file `state`, up to where it
/// would take over the descriptors the state names. Nothing is bound,
/// locked, accepted or started.
pub fn check(options: &Options, state: &Path) -> Result<()> {
    let state = reload::decode_state(&reload::read_check_file(state)?)?;
    crate::fail::point("supervisor-resume")?;
    let mut fds = Vec::new();
    if state.listener >= 0 {
        fds.push(state.listener);
    }
    if let Some((listener, lock, _)) = handed_over_socket(&state) {
        fds.extend([listener, lock]);
    }
    if fds.is_empty() {
        bail!("nothing to listen on: the state hands over no socket");
    }
    for pane in &state.panes {
        let info = pane
            .info
            .as_ref()
            .context("a pane without its description")?;
        info.pane_id
            .parse::<PaneId>()
            .with_context(|| format!("pane id {:?}", info.pane_id))?;
        if pane.pid == 0 {
            bail!("pane {} has no process", info.pane_id);
        }
        fds.extend([pane.to_pane, pane.from_pane]);
    }
    let named = fds.len();
    fds.sort_unstable();
    fds.dedup();
    if fds.first().is_some_and(|&fd| fd < 0) || fds.len() != named {
        bail!("the descriptors the state names do not add up");
    }
    if options.auth {
        auth::token().context("the access token")?;
    }
    Ok(())
}

/// The Unix socket the previous supervisor handed over, if it did.
fn handed_over_socket(state: &SupervisorState) -> Option<(RawFd, RawFd, PathBuf)> {
    match (state.local_listener, state.local_lock) {
        (Some(fd), Some(lock)) if !state.local_path.is_empty() => {
            Some((fd, lock, PathBuf::from(&state.local_path)))
        }
        _ => None,
    }
}

/// Take the Unix socket. One that cannot be taken stops the server if it is
/// `required`; otherwise the server goes on with its TCP port alone.
fn set_up_local(options: LocalOptions) -> Result<Option<local::Local>> {
    match local::claim(&options.path, options.lock_fd) {
        Ok(local) => Ok(Some(local)),
        Err(err) if options.required => {
            anyhow::bail!("the Unix socket {}: {err}", options.path.display())
        }
        Err(err) => {
            tracing::warn!(
                path = %options.path.display(),
                "no Unix socket for local clients: {err}"
            );
            Ok(None)
        }
    }
}

/// Bind the TCP port. One that cannot be had stops the server if it is
/// `required`; otherwise the server goes on with its Unix socket alone.
async fn bind_listener(options: &ListenOptions) -> Result<Option<TcpListener>> {
    let addr = options.addr;
    let err = match TcpListener::bind(addr).await {
        Ok(listener) => return Ok(Some(listener)),
        Err(err) => err,
    };
    let why = if err.kind() == std::io::ErrorKind::AddrInUse {
        format!("{addr} is taken: another tilrd, or another program, listens there")
    } else {
        format!("cannot listen on {addr}: {err}")
    };
    if options.required {
        anyhow::bail!("{why}; pick another port with --listen, or none with --no-listen");
    }
    tracing::warn!(
        "{why}; serving the Unix socket only (no Tilr app, remote clients or tilr Cloud tunnel through this server)"
    );
    Ok(None)
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

/// Listen for host events (see [`crate::host`]) and return the socket.
/// `None`, with a warning, if that cannot be done: the server works on
/// without them.
fn set_up_host_socket(
    server_id: &Uuid,
    events: mpsc::UnboundedSender<Event>,
) -> Option<host::Bound> {
    let server_id = server_id.to_string();
    host::sweep(&server_id);
    let path = host::choose_path(&server_id);
    match host::bind(&path) {
        Ok((listener, bound)) => {
            tracing::info!(path = %path.display(), "host events on");
            tokio::spawn(host::serve(
                listener,
                Box::new(move |request, reply| events.send(Event::Host { request, reply }).is_ok()),
            ));
            Some(bound)
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
    // The login shells' startup wrappers, which keep the tools first on PATH;
    // without them shells start as before (and stale ones must not be used).
    if let Err(err) = shell::install_wrappers(&runtime_dir) {
        tracing::warn!("shell startup wrappers are off: {err:#}");
        let _ = std::fs::remove_dir_all(shell::wrapper_dir(&runtime_dir));
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

async fn shutdown_signal(
    mut term: tokio::signal::unix::Signal,
    mut int: tokio::signal::unix::Signal,
    events: mpsc::UnboundedSender<Event>,
) {
    tokio::select! {
        _ = int.recv() => {}
        _ = term.recv() => {}
    }
    let _ = events.send(Event::Shutdown);
}

/// SIGUSR1 reloads the supervisor and the pane processes, to pick up a
/// rebuilt executable without closing any shell.
async fn reload_signal(
    mut usr1: tokio::signal::unix::Signal,
    events: mpsc::UnboundedSender<Event>,
) {
    while usr1.recv().await.is_some() {
        if events.send(Event::Reload).is_err() {
            break;
        }
    }
}

/// SIGHUP is the terminal the server was started from going away (an SSH
/// session dropping, the shell passing it on to its jobs). The server, like
/// its panes, outlives that terminal; reloading is SIGUSR1's.
async fn hangup_signal(mut hup: tokio::signal::unix::Signal) {
    if hup.recv().await.is_some() {
        tracing::info!("SIGHUP ignored: the server keeps running without its terminal");
    }
    while hup.recv().await.is_some() {}
}

/// Tick every [`LAG_TICK`] through the event loop, which measures how late
/// it gets to each.
async fn lag_ticks(events: mpsc::UnboundedSender<Event>) {
    let mut interval = tokio::time::interval(LAG_TICK);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        let due = interval.tick().await;
        if events.send(Event::Tick(due.into_std())).is_err() {
            break;
        }
    }
}

async fn accept_loop(
    listener: Arc<TcpListener>,
    door: Arc<Door>,
    client_timeout: Option<Duration>,
    metrics: Arc<metrics::Shared>,
    next_id: Arc<AtomicU64>,
    events: mpsc::UnboundedSender<Event>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let id: ClientId = next_id.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve_client(
                    id,
                    stream,
                    addr,
                    Arc::clone(&door),
                    client_timeout,
                    Arc::clone(&metrics),
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

/// Accept local clients on the Unix socket: this user's processes only.
async fn accept_local(
    listener: Arc<UnixListener>,
    door: Arc<Door>,
    client_timeout: Option<Duration>,
    metrics: Arc<metrics::Shared>,
    next_id: Arc<AtomicU64>,
    events: mpsc::UnboundedSender<Event>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let pid = match local::peer(&stream) {
                    Ok(pid) => pid,
                    Err(err) => {
                        tracing::warn!("refused a connection on the Unix socket: {err:#}");
                        continue;
                    }
                };
                let id: ClientId = next_id.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve_local(
                    id,
                    stream,
                    pid,
                    Arc::clone(&door),
                    client_timeout,
                    Arc::clone(&metrics),
                    events.clone(),
                ));
            }
            Err(err) => {
                tracing::warn!("accept on the Unix socket failed: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// A connection on the Unix socket: the main protocol only, no token.
async fn serve_local(
    id: ClientId,
    stream: UnixStream,
    pid: Option<i32>,
    door: Arc<Door>,
    client_timeout: Option<Duration>,
    shared_metrics: Arc<metrics::Shared>,
    events: mpsc::UnboundedSender<Event>,
) {
    let peer = Peer::Unix(pid);
    let socket = stream.as_raw_fd();
    serve_websocket(
        Connection {
            id,
            peer,
            local: true,
            socket,
        },
        stream,
        &door,
        client_timeout,
        shared_metrics,
        events,
    )
    .await;
}

/// One connection, as [`serve_websocket`] serves it.
struct Connection {
    id: ClientId,
    peer: Peer,
    /// On the Unix socket.
    local: bool,
    socket: RawFd,
}

async fn serve_client(
    id: ClientId,
    stream: TcpStream,
    addr: SocketAddr,
    door: Arc<Door>,
    client_timeout: Option<Duration>,
    shared_metrics: Arc<metrics::Shared>,
    events: mpsc::UnboundedSender<Event>,
) {
    // Keystrokes and small diffs must not wait for Nagle.
    let _ = stream.set_nodelay(true);
    // An HTTP request starts with a letter; a SOCKS5 client with its version.
    let mut first = [0u8; 1];
    let peeked = match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(peeked) => peeked,
        Err(_) => {
            tracing::debug!(%addr, "dropped a connection that said nothing in {HANDSHAKE_TIMEOUT:?}");
            return;
        }
    };
    if matches!(peeked, Ok(1)) && first[0] == socks::VERSION {
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
    // A plain HTTP request, not a WebSocket.
    if metrics::is_request(&stream).await {
        return serve_metrics(stream, addr, &door, &shared_metrics, &events).await;
    }
    let socket = stream.as_raw_fd();
    let connection = Connection {
        id,
        peer: Peer::Tcp(addr),
        local: false,
        socket,
    };
    serve_websocket(
        connection,
        stream,
        &door,
        client_timeout,
        shared_metrics,
        events,
    )
    .await;
}

/// The WebSocket on a connection: the main protocol, or SOCKS on TCP.
async fn serve_websocket<S>(
    connection: Connection,
    stream: S,
    door: &Door,
    client_timeout: Option<Duration>,
    shared_metrics: Arc<metrics::Shared>,
    events: mpsc::UnboundedSender<Event>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Connection {
        id,
        peer: addr,
        local,
        socket,
    } = connection;
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
        // SOCKS is for the TCP port, where a browser elsewhere comes in.
        if socks && (!door.socks || local) {
            return Err(refuse(StatusCode::NOT_FOUND, "not found"));
        }
        if !door.origin_allowed(request, addr) {
            return Err(refuse(StatusCode::FORBIDDEN, "origin not allowed"));
        }
        if socks {
            // The main protocol asks for the token in `Hello`; SOCKS has no
            // such message, so the handshake carries it.
            if !door.token_shown(request) {
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
    let handshake = tokio_tungstenite::accept_hdr_async(stream, admit);
    let ws = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(ws)) => ws,
        Err(_) => {
            tracing::debug!(%addr, "dropped a connection whose handshake took over {HANDSHAKE_TIMEOUT:?}");
            return;
        }
        // `tilr` looks whether the socket is served by connecting to it.
        Ok(Err(err)) if local => {
            tracing::debug!(%addr, "websocket handshake failed: {err}");
            return;
        }
        Ok(Err(err)) => {
            tracing::warn!(%addr, "websocket handshake failed: {err}");
            return;
        }
    };
    if route == Route::Socks {
        let Peer::Tcp(addr) = addr else { return };
        tracing::debug!(%addr, "SOCKS WebSocket connected");
        socks::serve_websocket(ws, client_timeout, addr).await;
        return;
    }
    tracing::info!(client = id, %addr, "client connected");
    if !local {
        limit_unsent(socket);
    }
    let (mut sink, mut source) = ws.split();
    let (tx, mut outbox) = outbox::channel();
    let (closer, mut closing) = mpsc::unbounded_channel();
    let connected = Event::ClientConnected {
        id,
        addr,
        local,
        tx,
        close: closer.clone(),
    };
    if events.send(connected).is_err() {
        return;
    }

    // A network that drops out without closing the connection leaves it open
    // for hours. Pings make a client that is still there answer, so one that
    // stays silent is known to be gone, and one that sends but no longer
    // answers is known not to hear the server.
    let mut ping = client_timeout.map(|timeout| {
        let mut ping = tokio::time::interval(timeout / PINGS_PER_TIMEOUT);
        ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ping
    });
    // Ends when the connection is to close (see `Closing`), with whether the
    // close went out.
    let drained = events.clone();
    let (mut window, acks) = outbox::BulkWindow::new();
    let mut writer = tokio::spawn(async move {
        let how = loop {
            let open = window.open();
            let (message, outgoing) = tokio::select! {
                // The supervisor says why before it lets go of the queue.
                biased;
                how = closing.recv() => break how,
                packet = outbox.recv(open) => match packet {
                    Some(mut outgoing) => {
                        let packet = std::mem::take(&mut outgoing.packet);
                        (Message::Binary(packet), Some(outgoing))
                    }
                    None => break closing.try_recv().ok(),
                },
                // It carries the bulk position, so that its pong moves the
                // window even if the client answers only the latest ping.
                () = tick(&mut ping) => (Message::Ping(window.position()), None),
                // The client read more: bulk data may go again.
                () = window.moved(), if !open => continue,
            };
            let mark = match (&message, &outgoing) {
                (Message::Binary(packet), Some(outgoing)) if outgoing.bulk() => {
                    window.wrote(packet.len())
                }
                _ => None,
            };
            let send = sink.send(message);
            tokio::pin!(send);
            tokio::select! {
                sent = &mut send => if sent.is_err() {
                    return false;
                },
                how = closing.recv() => {
                    // A close at once does not wait for a client that does
                    // not read; any other finishes what it was writing.
                    if !matches!(how, Some(Closing::Now(..))) {
                        let _ = tokio::time::timeout(CLOSE_TIMEOUT, send).await;
                    }
                    break how;
                }
            }
            // A ping after the bulk data it marks, whose pong says the client
            // has read that far.
            if let Some(mark) = mark
                && sink.send(Message::Ping(mark)).await.is_err()
            {
                return false;
            }
            if let Some(outgoing) = outgoing {
                if let Some(read_at) = outgoing.read_at {
                    shared_metrics.route.observe(read_at.elapsed());
                }
                if outbox.written(outgoing) {
                    let _ = drained.send(Event::ClientDrained { id });
                }
            }
        };
        let close = async {
            let frame = match how {
                Some(Closing::Now(code, reason)) => Some(CloseFrame {
                    code: code.into(),
                    reason: reason.into(),
                }),
                Some(Closing::Flush) => {
                    while let Some(Outgoing { packet, .. }) = outbox.try_recv() {
                        sink.send(Message::Binary(packet)).await?;
                    }
                    None
                }
                None => None,
            };
            sink.send(Message::Close(frame)).await
        };
        matches!(tokio::time::timeout(CLOSE_TIMEOUT, close).await, Ok(Ok(())))
    });

    let mut heard = tokio::time::Instant::now();
    let mut answered = heard;
    let why = loop {
        let mut wake = None;
        if let Some(timeout) = client_timeout {
            match judge(heard, answered, timeout) {
                Ok(next) => wake = Some(next),
                Err(why) => break Some(why),
            }
        }
        let message = tokio::select! {
            biased;
            // The supervisor closed the connection.
            closed = &mut writer => {
                finish_close(&mut source, socket, closed.unwrap_or(false)).await;
                break None;
            }
            message = source.next() => message,
            () = sleep_until(wake) => continue,
        };
        let Some(Ok(message)) = message else {
            break None;
        };
        heard = tokio::time::Instant::now();
        match message {
            Message::Binary(packet) => {
                let received = Instant::now();
                let event = Event::ClientPacket {
                    id,
                    packet,
                    received,
                };
                if events.send(event).is_err() {
                    break None;
                }
            }
            Message::Pong(payload) => {
                answered = heard;
                acks.pong(&payload);
            }
            Message::Close(_) => break None,
            _ => {}
        }
    };
    let _ = events.send(Event::ClientGone { id, why });
    if why.is_some() {
        let _ = closer.send(Closing::Now(close::TIMEOUT, close::TIMEOUT_REASON));
        if let Ok(closed) = writer.await {
            finish_close(&mut source, socket, closed).await;
        }
    } else if !writer.is_finished() {
        writer.abort();
    }
}

/// Whether a client is still there, from when anything was last `heard`
/// from it and when it last `answered` a ping: `Ok` with when to look again,
/// or why it is to be dropped. Silent: nothing heard for the timeout, the
/// way from it gone (or the client). Deaf: it still sends (within the last
/// ping interval) but answered no ping for the timeout, the way to it gone. A
/// client that went quiet only a while ago is given the rest of its timeout
/// as silent, not judged deaf for the pongs it could not send meanwhile.
fn judge(
    heard: tokio::time::Instant,
    answered: tokio::time::Instant,
    timeout: Duration,
) -> Result<tokio::time::Instant, ClientDrop> {
    let now = tokio::time::Instant::now();
    if now >= heard + timeout {
        return Err(ClientDrop::Silent);
    }
    let talking = now < heard + timeout / PINGS_PER_TIMEOUT;
    if now >= answered + timeout {
        if talking {
            return Err(ClientDrop::NoPong);
        }
        // Deaf as soon as it talks again (a message wakes the loop), silent
        // if it does not.
        return Ok(heard + timeout);
    }
    Ok((answered + timeout).min(heard + timeout))
}

/// After the close frame went out (`sent`), wait a moment for the client's
/// answer, which ends the connection cleanly. A close frame that could not
/// go out leaves the connection to be reset when dropped: what the socket
/// still holds for the client is stale, and a client that does not read
/// would hold it for minutes.
async fn finish_close<S>(source: &mut S, socket: RawFd, sent: bool)
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    if !sent {
        return reset_on_close(socket);
    }
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
        while let Some(Ok(message)) = source.next().await {
            if message.is_close() {
                break;
            }
        }
    })
    .await;
}

/// Sleep until `deadline`; forever without one.
async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Keep at most [`UNSENT_LIMIT`] bytes unsent in the client's socket. Best
/// effort: without it a slow client is only found later.
fn limit_unsent(socket: RawFd) {
    #[cfg(target_os = "linux")]
    const TCP_NOTSENT_LOWAT: libc::c_int = libc::TCP_NOTSENT_LOWAT;
    // <netinet/tcp.h>; the libc crate does not have it for Apple.
    #[cfg(target_os = "macos")]
    const TCP_NOTSENT_LOWAT: libc::c_int = 0x201;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    set_socket_option(socket, libc::IPPROTO_TCP, TCP_NOTSENT_LOWAT, &UNSENT_LIMIT);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = socket;
}

/// Have closing `socket` reset the connection instead of sending what is
/// left: a linger of zero.
fn reset_on_close(socket: RawFd) {
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    set_socket_option(socket, libc::SOL_SOCKET, libc::SO_LINGER, &linger);
}

fn set_socket_option<T>(socket: RawFd, level: libc::c_int, name: libc::c_int, value: &T) {
    // SAFETY: `value` points to a live T of the size passed; the socket is
    // the caller's, open for as long as the call.
    let status = unsafe {
        libc::setsockopt(
            socket,
            level,
            name,
            std::ptr::from_ref(value).cast(),
            std::mem::size_of::<T>() as libc::socklen_t,
        )
    };
    if status != 0 {
        tracing::debug!(
            level,
            name,
            "setsockopt failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// What a WebSocket on the listening port is for.
#[derive(PartialEq)]
enum Route {
    /// The main protocol, whatever the path.
    Main,
    /// The SOCKS proxy, at [`socks::PATH`].
    Socks,
}

impl Door {
    /// Whether a request may come from where it says it does. A browser
    /// attaches the page's origin to every request, WebSocket handshakes
    /// included. With no other authentication, this check is what stops an
    /// arbitrary website from opening a shell on this machine.
    fn origin_allowed(&self, request: &Request, addr: impl std::fmt::Display) -> bool {
        let Some(origin) = request.headers().get("origin") else {
            return true;
        };
        let origin = origin.to_str().unwrap_or("");
        let allowed = self.allow_origins.iter().any(|allowed| allowed == origin);
        if !allowed {
            tracing::warn!(%addr, origin, "rejected connection from disallowed origin");
        }
        allowed
    }

    /// Whether an HTTP request (`/socks`, `/metrics`) shows the access
    /// token, or the server asks for none.
    fn token_shown(&self, request: &Request) -> bool {
        self.token.as_ref().is_none_or(|token| {
            request_token(request).is_some_and(|given| auth::matches(token, given))
        })
    }

    /// Whether a request that needs no token (a server without one) may
    /// have the metrics. A browser page of another site could otherwise
    /// read them: `Sec-Fetch-Site: cross-site` says it is one, and a `Host`
    /// that is not this machine's loopback or the address listened on is
    /// a DNS rebinding.
    fn tokenless_metrics_allowed(&self, request: &Request, addr: SocketAddr) -> bool {
        let header = |name| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        if header("sec-fetch-site").is_some_and(|site| site.eq_ignore_ascii_case("cross-site")) {
            tracing::warn!(%addr, "rejected cross-site metrics request");
            return false;
        }
        let host = header("host").unwrap_or("");
        if !host_allowed(host, &self.listen_host) {
            tracing::warn!(%addr, host, "rejected metrics request for another host");
            return false;
        }
        true
    }
}

/// Whether `host` (a `Host` header: a name or address, maybe a port) is
/// loopback (`localhost`, `127.0.0.1`, `[::1]`) or `listen_host`, the
/// address the server listens on.
fn host_allowed(host: &str, listen_host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        // `[::1]:7890`
        Some(rest) => match rest.split_once(']') {
            Some((name, port)) if port.is_empty() || port.starts_with(':') => name,
            _ => return false,
        },
        None => host.split_once(':').map_or(host, |(name, _)| name),
    };
    if name.is_empty() {
        return false;
    }
    name.eq_ignore_ascii_case("localhost")
        || name == "127.0.0.1"
        || name == "::1"
        || name.eq_ignore_ascii_case(listen_host)
}

/// Answer `GET /metrics`: the token and the origin are checked as for
/// `/socks`, then the event loop is asked for its counters.
async fn serve_metrics(
    mut stream: TcpStream,
    addr: SocketAddr,
    door: &Door,
    shared: &metrics::Shared,
    events: &mpsc::UnboundedSender<Event>,
) {
    let Some(request) = metrics::read_request(&mut stream).await else {
        return metrics::respond(&mut stream, StatusCode::BAD_REQUEST, &[], "bad request\n").await;
    };
    if !door.origin_allowed(&request, addr) {
        return metrics::respond(
            &mut stream,
            StatusCode::FORBIDDEN,
            &[],
            "origin not allowed\n",
        )
        .await;
    }
    if door.token.is_none() && !door.tokenless_metrics_allowed(&request, addr) {
        return metrics::respond(&mut stream, StatusCode::FORBIDDEN, &[], "forbidden\n").await;
    }
    if !door.token_shown(&request) {
        tracing::warn!(%addr, "rejected metrics request without the access token");
        let headers = [("www-authenticate", "Bearer")];
        return metrics::respond(
            &mut stream,
            StatusCode::UNAUTHORIZED,
            &headers,
            "access token required\n",
        )
        .await;
    }
    let (reply, answer) = oneshot::channel();
    let snapshot = match events.send(Event::Metrics { reply }) {
        Ok(()) => tokio::time::timeout(METRICS_TIMEOUT, answer).await.ok(),
        Err(_) => None,
    };
    let Some(Ok(snapshot)) = snapshot else {
        let status = StatusCode::SERVICE_UNAVAILABLE;
        return metrics::respond(&mut stream, status, &[], "server busy\n").await;
    };
    let body = metrics::render(
        &snapshot,
        shared,
        std::process::id(),
        metrics::process_usage,
    );
    let headers = [("content-type", metrics::CONTENT_TYPE)];
    metrics::respond(&mut stream, StatusCode::OK, &headers, &body).await;
}

/// `HelloAck.seq_base` of a connection: its id in the high 32 bits, so every
/// connection's `ClientMessage.seq` (`seq_base + n`) is its own.
fn seq_base(client_id: ClientId) -> u64 {
    client_id << 32
}

/// The access token an HTTP request on the listening port carries (a SOCKS
/// WebSocket handshake, a metrics scrape): `Authorization: Bearer <token>`,
/// else `?token=<token>` (a page cannot set headers on a WebSocket).
fn request_token(request: &Request) -> Option<&str> {
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
    /// `None`: no TCP port, the Unix socket only.
    listener: Option<Arc<TcpListener>>,
    /// The Unix socket for local clients.
    local: Option<local::Local>,
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
    /// `None` when Exec is turned off.
    execs: Option<exec::Execs>,
    /// Clients may run programs (the "exec" feature); server commands run
    /// either way.
    exec: bool,
    /// The measurements the routing tasks share.
    metrics: Arc<metrics::Shared>,
    /// The id the next connection gets; handed on in a reload.
    next_client_id: Arc<AtomicU64>,
}

impl Supervisor {
    /// Returns the Unix socket, for the caller to remove once the panes are
    /// let go of.
    async fn run(mut self, mut events: mpsc::UnboundedReceiver<Event>) -> Option<local::Local> {
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
        // Closing the pipes to the pane processes makes them end what runs
        // in their panes (SIGHUP, and SIGKILL 2 s later for what ignores
        // it) and exit; the server waits for that, so nothing of a pane
        // outlives it. One still there after PANE_STOP is killed with its
        // handle.
        tracing::info!("shutting down, closing {} pane(s)", self.panes.len());
        self.stop_panes().await;
        // Asked to exit, so it signs off at Cloudflare, and reaped.
        self.tunnel.hold().await;
        if let Some(agent) = &mut self.agent {
            agent.remove_all();
        }
        if let Some(execs) = &mut self.execs {
            execs.stop_all(exec::End::Shutdown).await;
        }
        self.local.take()
    }

    /// Close the pipe to every pane process and wait, up to [`PANE_STOP`],
    /// until each has exited.
    async fn stop_panes(&mut self) {
        let deadline = tokio::time::Instant::now() + PANE_STOP;
        let mut readers = Vec::new();
        for (_, pane) in self.panes.drain() {
            drop(pane.to_pane);
            // The writer hands back the pipe once its queue is written;
            // dropping that closes it.
            drop(tokio::time::timeout_at(deadline, pane.link.writer).await);
            readers.push(pane.link.reader);
        }
        for reader in readers {
            // The reader waits for its process once the pipe from it ends.
            let _ = tokio::time::timeout_at(deadline, reader).await;
        }
    }

    fn on_event(&mut self, event: Event) {
        match event {
            Event::ClientConnected {
                id,
                addr,
                local,
                tx,
                close,
            } => {
                self.clients.insert(
                    id,
                    Client {
                        tx,
                        close,
                        addr,
                        local,
                        subscriptions: HashSet::new(),
                        connected: Instant::now(),
                        greeted: false,
                        host_events: false,
                        agent: false,
                        client_id: String::new(),
                        name: String::new(),
                        token: String::new(),
                        lagging: HashMap::new(),
                    },
                );
            }
            Event::ClientPacket {
                id,
                packet,
                received,
            } => self.on_client_packet(id, packet, received),
            Event::ClientGone { id, why } => {
                self.drop_client(id, why.map_or(Gone::Disconnected, Gone::Dropped));
            }
            Event::ClientDrained { id } => self.on_client_drained(id),
            Event::PaneStarted { pane_id } => self.on_pane_started(pane_id),
            Event::PaneOutput {
                pane_id,
                packet,
                read_at,
                head,
            } => self.on_pane_output(pane_id, packet, read_at, head),
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
            Event::Exec(exec::Output::Ended { exec_id }) => {
                if let Some(execs) = &mut self.execs {
                    execs.ended(exec_id);
                }
            }
            // An exec's reply did not fit the client's lane: dropped, as for
            // a frame, rather than left waiting for it.
            Event::Exec(exec::Output::Stuck { client }) => self.drop_stuck(client),
            Event::Host { request, reply } => {
                let _ = reply.send(self.on_host_event(request));
            }
            Event::Tick(due) => {
                self.metrics.loop_lag.observe(due.elapsed());
                self.ask_lagging_for_full_frames();
                self.close_ungreeted();
            }
            Event::Metrics { reply } => {
                let _ = reply.send(self.metrics_snapshot());
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
                process: pty::Process::inherited(pane.pid),
                from_pane,
                unparsed: BytesMut::from(pane.unparsed.as_slice()),
            };
            // The shell has the socket's path from before; it is made again
            // at the same place.
            if let Some(agent) = &mut self.agent {
                agent.listen_resumed(pane_id);
            }
            let (sender, link) = attach(
                pane_id,
                to_pane,
                detached,
                self.events.clone(),
                Arc::clone(&self.metrics),
            );
            // An empty packet is the reload request.
            let _ = sender.send((Bytes::new(), None));
            self.panes.insert(
                pane_id,
                Pane {
                    command: info.command,
                    title: info.title,
                    cwd: info.cwd,
                    created_at_ms: info.created_at_ms,
                    used_at_ms: info.used_at_ms.max(info.created_at_ms),
                    cols: info.cols.clamp(1, u32::from(u16::MAX)) as u16,
                    rows: info.rows.clamp(1, u32::from(u16::MAX)) as u16,
                    subscribers: HashSet::new(),
                    pid: pane.pid,
                    to_pane: sender,
                    link,
                    counters: metrics::PaneCounters::default(),
                    full_asked: false,
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
        let Some(formats) = self
            .while_serving(events, reload::check_executable(exe))
            .await
        else {
            return true;
        };
        let hand_over_socket = match formats {
            Ok(takes_socket) => takes_socket,
            Err(err) => {
                tracing::error!("not reloading: {err:#}");
                return false;
            }
        };
        // Without the socket the successor would leave its file behind with
        // nobody serving it, and the next `tilr` would start a second
        // server.
        if !hand_over_socket && self.local.is_some() {
            tracing::error!(
                "not reloading: {} cannot take over the Unix socket this server serves",
                exe.display()
            );
            return false;
        }

        // The successor shows it can resume from the state as it would be
        // left now, before anything is stopped.
        let panes = self
            .panes
            .iter()
            .map(|(&pane_id, pane)| (pane_id, pane.link.fds, Vec::new()))
            .collect::<Vec<_>>();
        let state = prost::Message::encode_to_vec(&self.handover_state(panes, hand_over_socket).0);
        let check = reload::check_successor(
            exe,
            std::env::args_os().skip(1).collect(),
            reload::check_path(),
            &state,
        );
        let Some(checked) = self.while_serving(events, check).await else {
            return true;
        };
        if let Err(err) = checked {
            tracing::error!("not reloading: the new executable could not resume: {err:#}");
            return false;
        }
        // A pane that started meanwhile is waited for, and checked with.
        if !self.starting.is_empty() {
            let events = self.events.clone();
            tokio::spawn(async move {
                tokio::time::sleep(RELOAD_RETRY).await;
                let _ = events.send(Event::Reload);
            });
            return false;
        }
        tracing::info!(exe = %exe.display(), panes = self.panes.len(), "reloading");

        // Execs are not handed over: they end here, and their clients are
        // told so before the connections go.
        if let Some(execs) = &mut self.execs
            && execs.stop_all(exec::End::Reload).await
        {
            self.flush_clients().await;
        }

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
                    // Waited for here if it goes at once: a child that
                    // exits after the exec is a zombie nobody waits for.
                    let mut process = reader.process;
                    if tokio::time::timeout(DETACH_TIMEOUT, process.wait())
                        .await
                        .is_err()
                    {
                        tokio::spawn(async move { process.wait().await });
                    }
                    self.on_pane_link_closed(pane_id, None, None);
                }
            }
        }

        // It is not carried over: the successor starts its own.
        self.tunnel.hold().await;
        let err = match crate::fail::point("reload-before-exec") {
            Ok(()) => self.exec_successor(&handed_over, hand_over_socket),
            Err(err) => err,
        };
        tracing::error!("reload failed: {err:#}");
        self.tunnel.release();
        // Carry on as before.
        for (pane_id, to_pane, reader) in handed_over {
            let (sender, link) = attach(
                pane_id,
                to_pane,
                reader,
                self.events.clone(),
                Arc::clone(&self.metrics),
            );
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.to_pane = sender;
                pane.link = link;
            }
        }
        false
    }

    /// Wait for `work` while handling the events that come meanwhile, so
    /// that clients and panes are served while a successor is checked.
    /// `None` if the server is to shut down. A reload asked for meanwhile is
    /// the one under way.
    async fn while_serving<T>(
        &mut self,
        events: &mut mpsc::UnboundedReceiver<Event>,
        work: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        tokio::pin!(work);
        loop {
            tokio::select! {
                done = &mut work => return Some(done),
                event = events.recv() => match event {
                    None | Some(Event::Shutdown) => return None,
                    Some(Event::Reload) => {}
                    Some(event) => self.on_event(event),
                },
            }
        }
    }

    /// What the successor is left: the listening sockets, the pipes to the
    /// pane processes (`(pane, (to, from), unparsed)`) and a description of
    /// the panes. Also the descriptors that must survive the exec.
    fn handover_state(
        &self,
        panes: Vec<(PaneId, (RawFd, RawFd), Vec<u8>)>,
        hand_over_socket: bool,
    ) -> (SupervisorState, Vec<RawFd>) {
        let listener = self
            .listener
            .as_ref()
            .map_or(-1, |listener| listener.as_raw_fd());
        let mut inherited: Vec<RawFd> = self
            .listener
            .as_ref()
            .map(|listener| listener.as_raw_fd())
            .into_iter()
            .collect();
        let local = self.local.as_ref().filter(|_| hand_over_socket);
        let (local_listener, local_lock) = local.map(local::Local::fds).unzip();
        inherited.extend(local_listener.into_iter().chain(local_lock));
        let state = SupervisorState {
            listener,
            local_listener,
            local_lock,
            local_path: local
                .map(|local| local.path().to_string_lossy().into_owned())
                .unwrap_or_default(),
            format: reload::STATE_FORMAT,
            next_client_id: self.next_client_id.load(Ordering::Relaxed),
            panes: panes
                .into_iter()
                .filter_map(|(pane_id, (to_pane, from_pane), unparsed)| {
                    let pane = self.panes.get(&pane_id)?;
                    inherited.extend([to_pane, from_pane]);
                    Some(PaneState {
                        info: Some(pane.info(pane_id)),
                        pid: pane.pid,
                        to_pane,
                        from_pane,
                        unparsed,
                    })
                })
                .collect(),
        };
        (state, inherited)
    }

    /// Exec the executable on disk, leaving it the listening socket, the
    /// pipes to the pane processes and a description of the panes. Returns
    /// only on failure, with every descriptor as it was.
    fn exec_successor(
        &self,
        panes: &[(PaneId, pipe::Sender, Detached)],
        hand_over_socket: bool,
    ) -> anyhow::Error {
        let panes = panes
            .iter()
            .map(|(pane_id, to_pane, reader)| {
                (
                    *pane_id,
                    (to_pane.as_raw_fd(), reader.from_pane.as_raw_fd()),
                    reader.unparsed.to_vec(),
                )
            })
            .collect();
        let (state, inherited) = self.handover_state(panes, hand_over_socket);
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
                let mut command = std::process::Command::new(exe);
                command.args(std::env::args_os().skip(1));
                // What comes between the exec and the successor's handlers
                // waits for them.
                reload::hold_signals(&mut command);
                let err = command.exec();
                reload::release_held_signals();
                anyhow::Error::new(err).context(format!("exec {}", exe.display()))
            }
            Err(err) => anyhow::Error::new(err).context("make descriptors inheritable"),
        };
        let _ = set_inheritable(false);
        let _ = std::fs::remove_file(file);
        err
    }

    fn on_client_packet(&mut self, client_id: ClientId, packet: Bytes, received: Instant) {
        // A client the supervisor has dropped may still have packets on the
        // way. Acting on them would subscribe a client nobody will ever
        // remove again.
        let Some(greeted) = self.clients.get(&client_id).map(|client| client.greeted) else {
            return;
        };
        let msg = match tilr_proto::decode::<ClientMessage>(&packet) {
            Ok(ClientMessage { msg: Some(msg), .. }) => msg,
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
            // The Unix socket is this user's alone: no token there.
            Msg::Hello(_)
                if self
                    .clients
                    .get(&client_id)
                    .is_some_and(|client| client.local) => {}
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
                self.supersede(client_id, &hello.client_id, &hello.token);
                if let Some(client) = self.clients.get_mut(&client_id) {
                    client.greeted = true;
                    client.host_events = hello.features.iter().any(|f| f == FEATURE_HOST_EVENTS);
                    client.agent = hello.features.iter().any(|f| f == FEATURE_AGENT);
                    tracing::info!(
                        client = client_id,
                        client_id = hello.client_id,
                        name = hello.client_name,
                        addr = %client.addr,
                        "client said hello"
                    );
                    client.client_id = hello.client_id;
                    client.name = hello.client_name;
                    client.token = hello.token;
                }
                let clipboard_max_bytes =
                    self.clipboard.as_ref().map(clipboard::Receiver::max_bytes);
                let fs_max_bytes = self.files.as_ref().map(files::Files::max_bytes);
                let features = [
                    (FEATURE_CLIPBOARD, clipboard_max_bytes.is_some()),
                    (FEATURE_FILES, fs_max_bytes.is_some()),
                    (FEATURE_HOST_EVENTS, self.host_events),
                    (FEATURE_AGENT, self.agent.is_some()),
                    (FEATURE_EXEC, self.exec),
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
                        seq_base: seq_base(client_id),
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
            Msg::ClosePane(close) => self.forward(client_id, &close.pane_id, packet, None),
            Msg::Subscribe(Subscribe { pane_id: named }) => {
                let Some((pane_id, pane)) = self.pane_mut(&named) else {
                    return self.send_no_pane(client_id, &named);
                };
                pane.subscribers.insert(client_id);
                pane.used_at_ms = now_ms();
                // The pane answers with a full frame.
                let _ = pane.to_pane.send((packet, None));
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
                        client.lagging.remove(&pane_id);
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
                let _ = pane.to_pane.send((packet, None));
                self.broadcast_pane_list();
            }
            Msg::Key(input) => self.forward(client_id, &input.pane_id, packet, Some(received)),
            Msg::Mouse(input) => self.forward(client_id, &input.pane_id, packet, Some(received)),
            Msg::Paste(input) => self.forward(client_id, &input.pane_id, packet, Some(received)),
            Msg::Raw(input) => self.forward(client_id, &input.pane_id, packet, Some(received)),
            Msg::Scroll(input) => self.forward(client_id, &input.pane_id, packet, Some(received)),
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
                Some(files) => {
                    if let Some(client) = self.clients.get(&client_id) {
                        files.on_message(client_id, &client.tx, msg);
                    }
                }
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
            Msg::ExecStart(start) if start.server_command => {
                let local = self
                    .clients
                    .get(&client_id)
                    .is_some_and(|client| client.local);
                let (Some(execs), Some(client), true) =
                    (&mut self.execs, self.clients.get(&client_id), local)
                else {
                    let refused = tilr_proto::ExecStarted {
                        request_id: start.request_id,
                        error: "server commands are taken on the Unix socket only".into(),
                        ..Default::default()
                    };
                    return self.send(client_id, server_message::Msg::ExecStarted(refused));
                };
                let mut argv = vec![crate::exe().to_string_lossy().into_owned()];
                argv.extend(start.argv);
                let start = tilr_proto::ExecStart { argv, ..start };
                if let Some(reply) = execs.on_message(
                    client_id,
                    &client.client_id,
                    &client.tx,
                    Msg::ExecStart(start),
                ) {
                    self.send(client_id, reply);
                }
            }
            Msg::ExecStart(start) if !self.exec => {
                tracing::debug!(client = client_id, "exec request while exec is off");
                let refused = tilr_proto::ExecStarted {
                    request_id: start.request_id,
                    error: "exec is turned off".into(),
                    ..Default::default()
                };
                self.send(client_id, server_message::Msg::ExecStarted(refused));
            }
            msg @ (Msg::ExecStart(_)
            | Msg::ExecInput(_)
            | Msg::ExecSignal(_)
            | Msg::ExecCancel(_)) => {
                let (Some(execs), Some(client)) = (&mut self.execs, self.clients.get(&client_id))
                else {
                    return tracing::debug!(client = client_id, "exec request while exec is off");
                };
                if let Some(reply) = execs.on_message(client_id, &client.client_id, &client.tx, msg)
                {
                    self.send(client_id, reply);
                }
            }
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
            if client.tx.try_send(packet.clone().into()).is_ok() {
                delivered += 1;
            } else {
                stuck.push(client_id);
            }
        }
        for client_id in stuck {
            self.drop_stuck(client_id);
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

    /// Pass a client packet to the pane process as-is. `input`: it is the
    /// client's input, read at that instant, and counted as such.
    fn forward(&mut self, client_id: ClientId, named: &str, packet: Bytes, input: Option<Instant>) {
        match self.pane_mut(named) {
            Some((_, pane)) => {
                if input.is_some() {
                    pane.counters.input += 1;
                    pane.counters.input_bytes += packet.len() as u64;
                    pane.used_at_ms = now_ms();
                }
                let _ = pane.to_pane.send((packet, input));
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
        let started = start_pane_process(
            pane_id,
            create,
            &cwd,
            cols,
            rows,
            &env,
            self.events.clone(),
            Arc::clone(&self.metrics),
        );
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
            created_at_ms: now_ms(),
            used_at_ms: now_ms(),
            cols,
            rows,
            subscribers: HashSet::new(),
            pid,
            to_pane,
            link,
            counters: metrics::PaneCounters::default(),
            full_asked: false,
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

    /// Pass a pane's packet on to its subscribers. A frame (`head`) is
    /// skipped for a client that lags behind (see [`ToClient::lagging`]): it
    /// gets no more frames of the pane until a full one, asked of the pane
    /// once its queue is written out, brings it the screen as it is then.
    /// Its own input echoed in the frames it missed is carried by that one.
    fn on_pane_output(
        &mut self,
        pane_id: PaneId,
        packet: Bytes,
        read_at: Instant,
        head: Option<FrameHead>,
    ) {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            return;
        };
        // What a pane process relays is its frames.
        pane.counters.frames += 1;
        pane.counters.frame_bytes += packet.len() as u64;
        if head.is_some_and(|head| head.full) {
            pane.full_asked = false;
        }
        let mut stuck = Vec::new();
        let mut ask_full = false;
        for &client_id in &pane.subscribers {
            let Some(client) = self.clients.get_mut(&client_id) else {
                continue;
            };
            let mut packet = packet.clone();
            if let Some(head) = head {
                let own = (head.last_input_seq >> 32 == client_id).then_some(head.last_input_seq);
                match client.lagging.get_mut(&pane_id) {
                    Some(lag) if !head.full => {
                        lag.own_seq = lag.own_seq.max(own.unwrap_or(0));
                        pane.counters.frames_skipped += 1;
                        ask_full |= client.tx.drained();
                        continue;
                    }
                    Some(_) => {
                        let lag = client.lagging.remove(&pane_id).unwrap_or_default();
                        let seq = lag.own_seq.max(own.unwrap_or(0));
                        if lag.own_seq != 0 && seq != head.last_input_seq {
                            packet = with_last_input_seq(&packet, seq).unwrap_or(packet);
                        }
                    }
                    None if !head.full && client.tx.lagging() => {
                        tracing::debug!(client = client_id, pane = %pane_id, "client lags, skipping frames");
                        client.lagging.insert(
                            pane_id,
                            Lag {
                                own_seq: own.unwrap_or(0),
                            },
                        );
                        client.tx.want_drained();
                        // The writer may have emptied the lane before the
                        // ask: then no `ClientDrained` comes, so ask now.
                        ask_full |= client.tx.drained();
                        pane.counters.frames_skipped += 1;
                        continue;
                    }
                    None => {}
                }
            }
            let outgoing = Outgoing::new(packet, Some(read_at));
            if client.tx.try_send(outgoing).is_ok() {
                pane.counters.frames_sent += 1;
            } else {
                stuck.push(client_id);
            }
        }
        if ask_full {
            ask_full_frame(pane_id, pane);
        }
        // A client that lost a diff has a corrupt screen; disconnecting it is
        // the honest outcome.
        for client_id in stuck {
            self.drop_stuck(client_id);
        }
    }

    /// A client that lagged has written out its queue: ask the panes it
    /// skipped for a full frame.
    fn on_client_drained(&mut self, client_id: ClientId) {
        let Some(client) = self.clients.get(&client_id) else {
            return;
        };
        for pane_id in client.lagging.keys() {
            if let Some(pane) = self.panes.get_mut(pane_id) {
                ask_full_frame(*pane_id, pane);
            }
        }
    }

    /// Every tick: a connection that has not said `Hello` within
    /// [`HELLO_TIMEOUT`] of its WebSocket opening is closed, whatever the
    /// client timeout, so an un-greeted one is held 20 s at most.
    fn close_ungreeted(&mut self) {
        let late: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, client)| !client.greeted && client.connected.elapsed() >= HELLO_TIMEOUT)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            self.drop_client(id, Gone::NoHello);
        }
    }

    /// Every tick: a lagging client whose queue is written out gets the full
    /// frames it waits for, even if its `ClientDrained` was missed and the
    /// pane prints nothing more. Asking twice costs nothing (`full_asked`).
    fn ask_lagging_for_full_frames(&mut self) {
        for client in self.clients.values() {
            if client.lagging.is_empty() || !client.tx.drained() {
                continue;
            }
            for pane_id in client.lagging.keys() {
                if let Some(pane) = self.panes.get_mut(pane_id) {
                    ask_full_frame(*pane_id, pane);
                }
            }
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
            client.lagging.remove(&pane_id);
            let _ = client.tx.try_send(packet.clone().into());
        }
        self.broadcast_pane_list();
    }

    /// Forget a client and end its connection, saying why.
    fn drop_client(&mut self, client_id: ClientId, why: Gone) {
        let Some(client) = self.clients.remove(&client_id) else {
            return;
        };
        let (id, addr) = (client.client_id.as_str(), client.addr);
        let closing = match why {
            Gone::Disconnected => {
                tracing::info!(client = client_id, client_id = id, %addr, "client disconnected");
                None
            }
            Gone::Refused => Some(Closing::Flush),
            Gone::NoHello => {
                tracing::warn!(
                    client = client_id,
                    %addr,
                    "client sent no Hello in {}s, dropping",
                    HELLO_TIMEOUT.as_secs()
                );
                Some(Closing::Now(close::TIMEOUT, close::TIMEOUT_REASON))
            }
            Gone::Dropped(drop) => {
                self.metrics.count_drop(drop);
                let (code, reason) = match drop {
                    ClientDrop::Superseded => {
                        tracing::info!(
                            client = client_id,
                            client_id = id,
                            %addr,
                            "client superseded by a newer connection of the same client, dropping"
                        );
                        (close::SUPERSEDED, close::SUPERSEDED_REASON)
                    }
                    ClientDrop::TooSlow => {
                        tracing::warn!(client = client_id, client_id = id, %addr, "client too slow, dropping");
                        (close::TOO_SLOW, close::TOO_SLOW_REASON)
                    }
                    ClientDrop::Silent => {
                        tracing::warn!(client = client_id, client_id = id, %addr, "client went silent, dropping");
                        (close::TIMEOUT, close::TIMEOUT_REASON)
                    }
                    ClientDrop::NoPong => {
                        tracing::warn!(
                            client = client_id,
                            client_id = id,
                            %addr,
                            "client stopped answering pings, dropping"
                        );
                        (close::TIMEOUT, close::TIMEOUT_REASON)
                    }
                };
                Some(Closing::Now(code, reason))
            }
        };
        // The connection's writer closes it; whatever else still holds the
        // queue's sender (an agent stream) does not keep it open.
        if let Some(closing) = closing {
            let _ = client.close.send(closing);
        }
        if let Some(clipboard) = &mut self.clipboard {
            clipboard.drop_client(client_id);
        }
        if let Some(files) = &mut self.files {
            files.drop_client(client_id);
        }
        if let Some(agent) = &mut self.agent {
            agent.drop_client(client_id);
        }
        if let Some(execs) = &mut self.execs {
            execs.drop_client(client_id);
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

    /// Drop a client whose queue took no more: too slow, unless its
    /// connection is already gone.
    fn drop_stuck(&mut self, client_id: ClientId) {
        let gone = self
            .clients
            .get(&client_id)
            .is_some_and(|client| client.tx.closed());
        let why = if gone {
            Gone::Disconnected
        } else {
            Gone::Dropped(ClientDrop::TooSlow)
        };
        self.drop_client(client_id, why);
    }

    /// A client says hello with `client_id`: close the connections it made
    /// before that are still open, under the same token. They are what a
    /// network the client left still holds (a half-open connection, a ghost
    /// viewer until the client timeout).
    fn supersede(&mut self, new: ClientId, client_id: &str, token: &str) {
        if client_id.is_empty() {
            return;
        }
        // A connection on the Unix socket supersedes only those, and one on
        // TCP only those: neither speaks for the other.
        let local = self.clients.get(&new).is_some_and(|client| client.local);
        let old: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|&(&id, client)| {
                id != new
                    && client.greeted
                    && client.local == local
                    && client.client_id == client_id
                    && auth::matches(&client.token, token)
            })
            .map(|(&id, _)| id)
            .collect();
        for id in old {
            if let (Some(old), Some(by)) = (self.clients.get(&id), self.clients.get(&new)) {
                tracing::info!(
                    client = id,
                    client_id,
                    addr = %old.addr,
                    by = new,
                    by_addr = %by.addr,
                    "superseding the client's older connection"
                );
            }
            self.drop_client(id, Gone::Dropped(ClientDrop::Superseded));
        }
    }

    /// The counters `/metrics` reports, panes oldest first.
    fn metrics_snapshot(&self) -> metrics::Snapshot {
        let mut panes: Vec<_> = self.panes.iter().collect();
        panes.sort_by_key(|(id, pane)| (pane.created_at_ms, **id));
        metrics::Snapshot {
            version: crate::VERSION.into(),
            server_id: self.server_id.to_string(),
            clients: self.clients.len(),
            panes: panes
                .into_iter()
                .map(|(id, pane)| metrics::PaneSnapshot {
                    id: id.to_string(),
                    pid: pane.pid,
                    counters: pane.counters,
                })
                .collect(),
        }
    }

    fn pane_list(&self, request_id: u64) -> PaneList {
        let mut panes: Vec<PaneInfo> = self
            .panes
            .iter()
            .map(|(&id, pane)| {
                let mut info = pane.info(id);
                info.viewed_by = pane
                    .subscribers
                    .iter()
                    .filter_map(|id| self.clients.get(id))
                    .map(|client| Viewer {
                        client_id: client.client_id.clone(),
                        client_name: client.name.clone(),
                        address: client.addr.to_string(),
                    })
                    .collect();
                info
            })
            .collect();
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
            .filter(|(_, client)| {
                client.greeted && client.tx.try_send(packet.clone().into()).is_err()
            })
            .map(|(&id, _)| id)
            .collect();
        for client_id in stuck {
            self.drop_stuck(client_id);
        }
    }

    /// Wait, up to [`FLUSH_TIMEOUT`], until everything queued for the
    /// clients is written out.
    async fn flush_clients(&self) {
        let deadline = tokio::time::Instant::now() + FLUSH_TIMEOUT;
        while tokio::time::Instant::now() < deadline
            && self
                .clients
                .values()
                .any(|client| !client.tx.closed() && !client.tx.idle())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn send(&mut self, client_id: ClientId, msg: server_message::Msg) {
        let packet = tilr_proto::encode(&ServerMessage::from(msg));
        let stuck = self
            .clients
            .get(&client_id)
            .is_some_and(|client| client.tx.try_send(packet.into()).is_err());
        if stuck {
            self.drop_stuck(client_id);
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
        self.drop_client(client_id, Gone::Refused);
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
            used_at_ms: self.used_at_ms,
            pid: self.pid,
            // Filled in by `Supervisor::pane_list`, which knows the clients.
            viewed_by: Vec::new(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// Ask the pane for a full frame, for the clients that skipped its frames,
/// unless one is asked for already. A subscribe is what makes a pane send one.
fn ask_full_frame(pane_id: PaneId, pane: &mut Pane) {
    if pane.full_asked {
        return;
    }
    pane.full_asked = true;
    let ask = ClientMessage::from(Msg::Subscribe(Subscribe {
        pane_id: pane_id.to_string(),
    }));
    let _ = pane.to_pane.send((tilr_proto::encode(&ask), None));
}

/// A frame packet with `Frame.last_input_seq` set to `seq`; `None` if it is
/// not a frame.
fn with_last_input_seq(packet: &[u8], seq: u64) -> Option<Bytes> {
    let mut msg = tilr_proto::decode::<ServerMessage>(packet).ok()?;
    let Some(server_message::Msg::Frame(Frame { last_input_seq, .. })) = &mut msg.msg else {
        return None;
    };
    *last_input_seq = seq;
    Some(tilr_proto::encode(&msg))
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
#[expect(
    clippy::too_many_arguments,
    reason = "all of it describes the new pane"
)]
fn start_pane_process(
    pane_id: PaneId,
    create: &CreatePane,
    cwd: &str,
    cols: u16,
    rows: u16,
    env: &[(OsString, OsString)],
    events: mpsc::UnboundedSender<Event>,
    shared_metrics: Arc<metrics::Shared>,
) -> Result<(u32, mpsc::UnboundedSender<ToPane>, Link)> {
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
    // A session of its own, away from the terminal the server may have been
    // started from: what goes to that terminal's process group (SIGHUP when
    // it hangs up, SIGINT on Ctrl-C) is not the pane's business. The
    // supervisor ends the pane by closing its stdin.
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
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
    let (sender, link) = attach(pane_id, to_pane, detached, events, shared_metrics);
    Ok((pid, sender, link))
}

/// Start the tasks that move packets to and from a pane process.
fn attach(
    pane_id: PaneId,
    to_pane: pipe::Sender,
    reader: Detached,
    events: mpsc::UnboundedSender<Event>,
    shared_metrics: Arc<metrics::Shared>,
) -> (mpsc::UnboundedSender<ToPane>, Link) {
    let (sender, outgoing) = mpsc::unbounded_channel();
    let stop_reader = Arc::new(Notify::new());
    let fds = (to_pane.as_raw_fd(), reader.from_pane.as_raw_fd());
    let link = Link {
        fds,
        writer: tokio::spawn(write_pane(to_pane, outgoing, shared_metrics)),
        reader: tokio::spawn(read_pane(pane_id, reader, Arc::clone(&stop_reader), events)),
        stop_reader,
    };
    (sender, link)
}

/// Write queued packets to the pane process until the queue's sender is
/// dropped, then return the pipe. `None` if the process stopped reading.
async fn write_pane(
    mut to_pane: pipe::Sender,
    mut outgoing: mpsc::UnboundedReceiver<ToPane>,
    shared_metrics: Arc<metrics::Shared>,
) -> Option<pipe::Sender> {
    while let Some((packet, received)) = outgoing.recv().await {
        link::write_packet(&mut to_pane, &packet).await.ok()?;
        if let Some(received) = received {
            shared_metrics.route_input.observe(received.elapsed());
        }
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
                link::kind::RELAY => Event::PaneOutput {
                    pane_id,
                    packet,
                    read_at: Instant::now(),
                    head: None,
                },
                link::kind::FRAME => match FrameHead::split(packet) {
                    Some((head, packet)) => Event::PaneOutput {
                        pane_id,
                        packet,
                        read_at: Instant::now(),
                        head: Some(head),
                    },
                    None => continue,
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_is_judged_silent_or_deaf_by_what_it_sends_and_answers() {
        let timeout = Duration::from_secs(30);
        let now = tokio::time::Instant::now();
        let ago = |secs| now - Duration::from_secs(secs);
        // Answers, talks: look again when the first of the two runs out.
        assert_eq!(judge(ago(1), ago(5), timeout), Ok(ago(5) + timeout));
        // Nothing heard for the timeout.
        assert_eq!(judge(ago(31), ago(31), timeout), Err(ClientDrop::Silent));
        // Still talking, no pong for the timeout: the way to it is gone.
        assert_eq!(judge(ago(1), ago(31), timeout), Err(ClientDrop::NoPong));
        // Quiet for a while and no pong: the rest of its timeout as silent.
        assert_eq!(judge(ago(20), ago(31), timeout), Ok(ago(20) + timeout));
    }

    #[test]
    fn metrics_without_a_token_are_for_this_machine_only() {
        for host in [
            "localhost",
            "LOCALHOST:7890",
            "127.0.0.1",
            "127.0.0.1:7890",
            "[::1]",
            "[::1]:7890",
            "10.0.0.5:7890",
        ] {
            assert!(host_allowed(host, "10.0.0.5"), "{host}");
        }
        for host in [
            "",
            "evil.example",
            "evil.example:7890",
            "localhost.evil.example",
            "127.0.0.2",
            "[::2]:7890",
            "[::1",
            "[::1]x",
            "10.0.0.5:7890",
        ] {
            assert!(!host_allowed(host, ""), "{host}");
        }
    }
}
