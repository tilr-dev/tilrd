//! Spawning and supervising the instances of `bus.toml` (B10): own session, fd 3 or
//! an instance socket, restarts with backoff, attached to the broker with
//! `BrokerHandle::attach_stream`.
//!
//! Owner: `bus-instances`.
//!
//! # Shape
//!
//! [`Runner`] is a handle (clones share it) on a table of instances. Each instance that
//! is meant to run has one *lifecycle task* ([`Life`]): it starts the process, waits for
//! its `Hello`, watches the process and the connection, and starts it again after a
//! pause when either ends. The control methods (`control.rs`) talk to a lifecycle task by
//! [`Cmd`]s; the operations that change something (`start`, `stop`, `reload`,
//! `reload_config`) take their turn in one queue (a fair mutex), so two never interleave,
//! while the listings read the table at any time, a reload in progress included.
//!
//! # The process
//!
//! A new session (`setsid`, so the process group is its own and a signal reaches what
//! the instance started), stdin `/dev/null`, stdout and stderr into the log line by line,
//! no descriptor of the server but its own bus connection: descriptor 3 (`connect =
//! "fd"`, one end of a socket pair), or a listening socket in the runtime directory
//! (`connect = "socket"`) whose first `Hello` with the one-time token is the instance.
//!
//! # Stopping
//!
//! SIGTERM to the group, SIGKILL after [`KILL_AFTER`], and a SIGKILL to what is left of
//! the group when the leader is gone. The group is signalled only while its leader is
//! not reaped: its exit is waited for with `WNOWAIT` ([`exec::exited`]), the group
//! killed, then the leader reaped, so a reused pid is never hit. A planned stop first
//! drains the connection (`BrokerHandle::drain_connection`): no new calls by method, the
//! ones in flight answered; the drain ends at once when the server exits.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tilr_bus::bus::{ConnectionError, Hello};
use tilr_bus::{BusError, codes, env as bus_env};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UnixListener;
use tokio::process::{Child, Command};
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::broker::{Attached, BrokerHandle, ConnId, ConnState, Ended};
use super::config::{self, BusConfig, Connect, InstanceSpec, Module};
use super::core::Core;
use super::identity::{Identity, Peer, Seat, Transport};
use crate::exec;
use crate::supervisor::Supervisor;

/// The pause before the first restart; it doubles up to [`BACKOFF_MAX`].
pub const BACKOFF_MIN: Duration = Duration::from_millis(200);
pub const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// An instance that ran this long starts its next pause from [`BACKOFF_MIN`] again.
pub const STABLE: Duration = Duration::from_secs(10);
/// A process says `Hello` within this long of its start, or it is killed and started again.
pub const HELLO_LIMIT: Duration = Duration::from_secs(30);
/// A connection on an instance socket that has said no valid `Hello` by then is closed.
pub const SOCKET_HELLO_LIMIT: Duration = Duration::from_secs(10);
/// SIGKILL follows SIGTERM after this long.
pub const KILL_AFTER: Duration = Duration::from_secs(3);
/// A caller that acts on itself gets its reply out before the action starts.
pub const REPLY_GRACE: Duration = Duration::from_millis(100);
/// A line an instance writes is logged up to this many bytes.
pub const LINE_MAX: usize = 64 << 10;

/// Variables of a pane's environment: a process the server starts is not in a pane.
fn pane_variables() -> [&'static str; 10] {
    [
        crate::pty::PANE_ENV,
        tilr_proto::local::SOCKET_ENV,
        crate::host::SOCK_ENV,
        crate::host::SERVER_ENV,
        crate::clipboard::SHIMS_ENV,
        crate::clipboard::DIR_ENV,
        crate::clipboard::MAX_ENV,
        crate::clipboard::TTL_ENV,
        crate::shell::ZDOTDIR_ENV,
        crate::shell::HISTFILE_ENV,
    ]
}

/// A deadline for a `Hello`, or what a test sets in the environment variable `var`
/// (milliseconds; only in a build with `failpoints`).
fn hello_limit(default: Duration, var: &str) -> Duration {
    #[cfg(feature = "failpoints")]
    {
        if let Some(ms) = std::env::var(var).ok().and_then(|value| value.parse().ok()) {
            return Duration::from_millis(ms);
        }
    }
    #[cfg(not(feature = "failpoints"))]
    let _ = var;
    default
}

/// How long a process has to say `Hello` (`TILR_TEST_BUS_HELLO_MS` in a test).
fn process_hello_limit() -> Duration {
    hello_limit(HELLO_LIMIT, "TILR_TEST_BUS_HELLO_MS")
}

/// How long a connection on an instance socket has to say `Hello`
/// (`TILR_TEST_BUS_SOCKET_HELLO_MS` in a test).
fn socket_hello_limit() -> Duration {
    hello_limit(SOCKET_HELLO_LIMIT, "TILR_TEST_BUS_SOCKET_HELLO_MS")
}

/// What an instance is doing, as `bus.instances` lists it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Between a process and the next (the pause before a restart).
    Down,
    /// Started, no `Hello` yet.
    Starting,
    Up,
    /// Being stopped or replaced: no new calls by method.
    Draining,
    /// Not meant to run: `autostart = false`, `bus.stop`.
    #[default]
    Stopped,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Down => "down",
            Self::Starting => "starting",
            Self::Up => "up",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
        }
    }
}

/// One instance for a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub module: String,
    pub transport: &'static str,
    pub state: State,
    /// 0 without a process.
    pub pid: u32,
    pub restarts: u32,
    /// The broker's id of its connection while it is up.
    pub connection: Option<ConnId>,
}

/// Which instances a `bus.reload` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Instance(String),
    Module(String),
    /// Every instance that runs.
    All,
}

/// What `bus.reload` did to one instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reloaded {
    pub instance: String,
    pub old_pid: u32,
    /// 0: not known yet (the caller reloaded itself and has its reply first).
    pub new_pid: u32,
}

/// What `bus.reload_config` did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigOutcome {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub reloaded: Vec<String>,
    pub kept: Vec<String>,
}

/// The runner, as the control methods and the supervisor's hooks hold it.
#[derive(Clone)]
pub struct Runner(Arc<Shared>);

struct Shared {
    broker: BrokerHandle,
    /// `None`: the environment names no configuration directory, so no instances.
    config_dir: Option<PathBuf>,
    runtime_dir: PathBuf,
    table: Mutex<Table>,
    /// The one queue of the operations that change something.
    queue: tokio::sync::Mutex<()>,
    /// True while the server exits or execs: a drain in progress ends at once.
    exit: watch::Sender<bool>,
}

#[derive(Default)]
struct Table {
    config: BusConfig,
    instances: BTreeMap<String, Entry>,
    /// The server is going down or exec'ing: nothing starts.
    exiting: bool,
}

struct Entry {
    spec: InstanceSpec,
    module: Module,
    /// Meant to run: `autostart`, or started by hand; false after `bus.stop`.
    wanted: bool,
    /// The lifecycle task, while there is one.
    life: Option<mpsc::UnboundedSender<Cmd>>,
    snap: Snap,
}

#[derive(Clone, Default)]
struct Snap {
    state: State,
    pid: u32,
    restarts: u32,
    conn: Option<ConnId>,
}

type StartResult = Result<u32, BusError>;

/// What a lifecycle task is asked.
enum Cmd {
    /// Answer with the pid once the instance said `Hello`.
    Wait(oneshot::Sender<StartResult>),
    /// Drain, stop and end the task.
    Stop {
        drain: Duration,
        done: oneshot::Sender<()>,
    },
    /// Drain, stop and start again at once; answer `(old pid, new pid)`.
    Reload {
        drain: Duration,
        done: oneshot::Sender<Result<(u32, u32), BusError>>,
    },
}

fn not_found(name: &str) -> BusError {
    BusError::new(
        codes::NOT_FOUND,
        format!("no instance {name:?} in bus.toml"),
    )
}

fn shutting_down() -> BusError {
    BusError::unavailable("tilrd is shutting down or reloading: no instance is started now")
}

impl Runner {
    pub fn new(broker: BrokerHandle, config_dir: Option<PathBuf>, runtime_dir: PathBuf) -> Self {
        Self(Arc::new(Shared {
            broker,
            config_dir,
            runtime_dir,
            table: Mutex::new(Table::default()),
            queue: tokio::sync::Mutex::new(()),
            exit: watch::Sender::new(false),
        }))
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        self.0.table.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// `bus.toml` of the configuration directory; without a directory, no instances.
    fn load(&self) -> Result<BusConfig, config::ConfigError> {
        match &self.0.config_dir {
            Some(dir) => config::load(dir, &self.0.runtime_dir),
            None => Ok(BusConfig::default()),
        }
    }

    /// `bus.toml` now: read at start. A bad file is logged and leaves no instances.
    pub fn start_from_file(&self) {
        match self.load() {
            Ok(config) => self.install(config),
            Err(err) => {
                tracing::error!(target: "tilrd::bus", "no instances are started: {err}");
            }
        }
    }

    /// Take `config` as the first and start its `autostart` instances.
    pub fn install(&self, config: BusConfig) {
        self.declare(&config);
        let names: Vec<String> = {
            let mut table = self.table();
            table.instances.clear();
            for (name, spec) in &config.instances {
                table.instances.insert(
                    name.clone(),
                    Entry::new(spec.clone(), config.modules[&spec.module].clone()),
                );
            }
            table.config = config;
            table
                .instances
                .iter()
                .filter(|(_, entry)| entry.wanted)
                .map(|(name, _)| name.clone())
                .collect()
        };
        for name in names {
            // A start that fails is retried by its lifecycle; nobody waits here.
            let _ = self.begin_start(&name, false);
        }
    }

    /// Hand the broker what `config` says of every call: `call_timeout_ms` (the
    /// deadline of a call that names none and the most one names; 0: none), and the
    /// methods its instances' `provide` rights name exactly, so a call of one with no
    /// provider live is `unavailable`, not `not_found` (a pattern names no method; an
    /// instance's own methods are known from its first `Hello` on).
    fn declare(&self, config: &BusConfig) {
        let broker = &self.0.broker;
        broker.set_call_timeout(Duration::from_millis(config.call_timeout_ms));
        broker.know_methods(
            config
                .instances
                .values()
                .flat_map(|spec| &spec.rights.provide)
                .filter(|right| !right.ends_with('*'))
                .cloned(),
        );
    }

    /// The instances, in name order.
    pub fn rows(&self) -> Vec<Row> {
        self.table()
            .instances
            .iter()
            .map(|(name, entry)| Row {
                name: name.clone(),
                module: entry.spec.module.clone(),
                transport: entry.spec.connect.as_str(),
                state: entry.snap.state,
                pid: entry.snap.pid,
                restarts: entry.snap.restarts,
                connection: entry.snap.conn,
            })
            .collect()
    }

    /// `drain_ms` of `bus.toml`.
    pub fn drain(&self) -> Duration {
        Duration::from_millis(self.table().config.drain_ms)
    }

    /// The instances `what` names, in name order; `not_found` for one that is not in
    /// `bus.toml`. Of a module or `All` only the ones that run (or are on their way).
    pub fn resolve(&self, what: &Target) -> Result<Vec<String>, BusError> {
        let table = self.table();
        match what {
            Target::Instance(name) => {
                if table.instances.contains_key(name) {
                    Ok(vec![name.clone()])
                } else {
                    Err(not_found(name))
                }
            }
            Target::Module(module) => {
                if !table.config.modules.contains_key(module) {
                    return Err(BusError::new(
                        codes::NOT_FOUND,
                        format!("no module {module:?} in bus.toml"),
                    ));
                }
                Ok(table
                    .instances
                    .iter()
                    .filter(|(_, entry)| entry.spec.module == *module && entry.life.is_some())
                    .map(|(name, _)| name.clone())
                    .collect())
            }
            Target::All => Ok(table
                .instances
                .iter()
                .filter(|(_, entry)| entry.life.is_some())
                .map(|(name, _)| name.clone())
                .collect()),
        }
    }

    /// The names of every configured instance.
    pub fn names(&self) -> Vec<String> {
        self.table().instances.keys().cloned().collect()
    }

    /// The instance `name` as listed, if it is in `bus.toml`.
    pub fn row(&self, name: &str) -> Option<Row> {
        self.rows().into_iter().find(|row| row.name == name)
    }

    /// Run `op` in the queue, after `delay`, on a task of its own: the caller going
    /// away does not stop it halfway.
    fn queued<T, Fut>(
        &self,
        delay: Duration,
        op: impl FnOnce(Runner) -> Fut + Send + 'static,
    ) -> impl Future<Output = Result<T, BusError>> + Send + 'static
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, BusError>> + Send + 'static,
    {
        let runner = self.clone();
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let _turn = runner.0.queue.lock().await;
            let result = op(runner.clone()).await;
            let _ = tx.send(result);
        });
        async move { rx.await.unwrap_or_else(|_| Err(shutting_down())) }
    }

    /// `bus.start`: start `name` and answer its pid once it said `Hello`.
    pub fn start(
        &self,
        name: String,
    ) -> impl Future<Output = Result<u32, BusError>> + Send + 'static {
        self.queued(Duration::ZERO, move |runner| async move {
            let started = runner.begin_start(&name, true)?;
            started.await.unwrap_or_else(|_| Err(shutting_down()))
        })
    }

    /// `bus.stop`: drain `name` for `drain` (`bus.toml`'s `drain_ms` unless given), stop
    /// it and keep it stopped. `after` is the pause before it starts: a caller that
    /// stops itself has its reply first.
    pub fn stop(
        &self,
        name: String,
        drain: Option<Duration>,
        after: Duration,
    ) -> impl Future<Output = Result<(), BusError>> + Send + 'static {
        self.queued(after, move |runner| async move {
            let drain = drain.unwrap_or_else(|| runner.drain());
            runner.stop_now(&name, drain).await
        })
    }

    /// `bus.reload`: restart `names` one at a time, each drained and waited for until
    /// its `Hello`. `after` as for [`stop`](Self::stop).
    pub fn reload(
        &self,
        names: Vec<String>,
        after: Duration,
    ) -> impl Future<Output = Result<Vec<Reloaded>, BusError>> + Send + 'static {
        self.queued(after, move |runner| async move {
            let drain = runner.drain();
            let mut reloaded = Vec::new();
            for name in names {
                let (old_pid, new_pid) = runner.reload_one(&name, drain).await?;
                reloaded.push(Reloaded {
                    instance: name,
                    old_pid,
                    new_pid,
                });
            }
            Ok(reloaded)
        })
    }

    /// `bus.reload_config`: read `bus.toml` again and apply the difference. `check` is
    /// asked for each instance the file adds, changes or removes (for every configured
    /// one when the file is bad) before anything changes; its first error answers.
    pub fn reload_config(
        &self,
        check: impl Fn(&str) -> Result<(), BusError> + Send + Sync + 'static,
    ) -> impl Future<Output = Result<ConfigOutcome, BusError>> + Send + 'static {
        self.queued(Duration::ZERO, move |runner| async move {
            runner.reload_config_now(&check).await
        })
    }

    /// Before the server execs or exits: stop every instance (SIGTERM to its group,
    /// SIGKILL after [`KILL_AFTER`]) and start nothing until [`resume_all`](Self::resume_all).
    /// A drain in progress (`bus.stop`, `bus.reload`) ends at once.
    pub async fn stop_all(&self) {
        self.0.exit.send_replace(true);
        let lives: Vec<_> = {
            let mut table = self.table();
            table.exiting = true;
            table
                .instances
                .values()
                .filter_map(|entry| entry.life.clone())
                .collect()
        };
        let mut done = Vec::new();
        for life in lives {
            let (tx, rx) = oneshot::channel();
            if life
                .send(Cmd::Stop {
                    drain: Duration::ZERO,
                    done: tx,
                })
                .is_ok()
            {
                done.push(rx);
            }
        }
        for rx in done {
            let _ = rx.await;
        }
    }

    /// A reload that did not happen: start again what [`stop_all`](Self::stop_all) stopped.
    pub fn resume_all(&self) {
        self.0.exit.send_replace(false);
        let names: Vec<String> = {
            let mut table = self.table();
            table.exiting = false;
            table
                .instances
                .iter()
                .filter(|(_, entry)| entry.wanted && entry.life.is_none())
                .map(|(name, _)| name.clone())
                .collect()
        };
        for name in names {
            let _ = self.begin_start(&name, false);
        }
    }

    /// Start `name`'s lifecycle task, or ask the running one to report; the receiver
    /// gets the pid once the instance said `Hello`, or why it did not.
    fn begin_start(
        &self,
        name: &str,
        mark_wanted: bool,
    ) -> Result<oneshot::Receiver<StartResult>, BusError> {
        let (tx, rx) = oneshot::channel();
        let mut table = self.table();
        if table.exiting {
            return Err(shutting_down());
        }
        let entry = table
            .instances
            .get_mut(name)
            .ok_or_else(|| not_found(name))?;
        if mark_wanted {
            entry.wanted = true;
        }
        if let Some(life) = &entry.life {
            if entry.snap.state == State::Draining {
                return Err(BusError::unavailable(format!("{name} is being stopped")));
            }
            life.send(Cmd::Wait(tx))
                .map_err(|_| BusError::unavailable(format!("{name} is being stopped")))?;
            return Ok(rx);
        }
        let (life_tx, life_rx) = mpsc::unbounded_channel();
        entry.life = Some(life_tx);
        entry.snap.state = State::Starting;
        let life = Life {
            runner: self.clone(),
            name: name.to_owned(),
            cmds: life_rx,
            waiters: vec![tx],
            reload: None,
        };
        tokio::spawn(life.run());
        Ok(rx)
    }

    async fn stop_now(&self, name: &str, drain: Duration) -> Result<(), BusError> {
        let life = {
            let mut table = self.table();
            let entry = table
                .instances
                .get_mut(name)
                .ok_or_else(|| not_found(name))?;
            entry.wanted = false;
            entry.life.clone()
        };
        let Some(life) = life else {
            return Ok(());
        };
        let (done, stopped) = oneshot::channel();
        if life.send(Cmd::Stop { drain, done }).is_ok() {
            let _ = stopped.await;
        }
        Ok(())
    }

    async fn reload_one(&self, name: &str, drain: Duration) -> Result<(u32, u32), BusError> {
        let life = {
            let table = self.table();
            let entry = table.instances.get(name).ok_or_else(|| not_found(name))?;
            entry.life.clone()
        };
        let Some(life) = life else {
            return Err(BusError::bad_request(format!(
                "{name} is stopped: start it with bus.start"
            )));
        };
        let (done, reloaded) = oneshot::channel();
        life.send(Cmd::Reload { drain, done })
            .map_err(|_| BusError::unavailable(format!("{name} is being stopped")))?;
        reloaded.await.unwrap_or_else(|_| {
            Err(BusError::unavailable(format!(
                "the reload of {name} was replaced"
            )))
        })
    }

    async fn reload_config_now(
        &self,
        check: &(dyn Fn(&str) -> Result<(), BusError> + Send + Sync),
    ) -> Result<ConfigOutcome, BusError> {
        let new = match self.load() {
            Ok(new) => new,
            Err(err) => {
                // A caller without the right learns nothing of the file.
                self.names().iter().try_for_each(|name| check(name))?;
                tracing::error!(target: "tilrd::bus", "bus.toml not applied: {err}");
                return Err(BusError::new(codes::BAD_CONFIG, err.0));
            }
        };
        let (diff, drain) = {
            let mut table = self.table();
            if table.exiting {
                return Err(shutting_down());
            }
            let diff = config::diff(&table.config, &new);
            diff.removed
                .iter()
                .chain(&diff.changed)
                .chain(&diff.added)
                .try_for_each(|name| check(name))?;
            table.config = new.clone();
            for name in diff.added.iter().chain(&diff.changed).chain(&diff.kept) {
                let spec = new.instances[name].clone();
                let module = new.modules[&spec.module].clone();
                match table.instances.get_mut(name) {
                    Some(entry) => {
                        entry.spec = spec;
                        entry.module = module;
                    }
                    None => {
                        table
                            .instances
                            .insert(name.clone(), Entry::new(spec, module));
                    }
                }
            }
            (diff, Duration::from_millis(table.config.drain_ms))
        };
        self.declare(&new);
        let mut outcome = ConfigOutcome::default();
        // Removed instances stop first, together.
        let mut stopping = Vec::new();
        for name in &diff.removed {
            let life = self.table().instances.get_mut(name).and_then(|entry| {
                entry.wanted = false;
                entry.life.clone()
            });
            if let Some(life) = life {
                let (done, stopped) = oneshot::channel();
                if life.send(Cmd::Stop { drain, done }).is_ok() {
                    stopping.push(stopped);
                }
            }
            outcome.stopped.push(name.clone());
        }
        for stopped in stopping {
            let _ = stopped.await;
        }
        {
            let mut table = self.table();
            for name in &diff.removed {
                table.instances.remove(name);
            }
        }
        // Changed ones, one at a time; a stopped one keeps its place and takes the new
        // entry when it is started.
        // The first error answers the call; `failed` names every instance that did not
        // come up, so the answer can say what was applied all the same.
        let mut failure = None;
        let mut failed = Vec::new();
        for name in &diff.changed {
            let runs = self
                .table()
                .instances
                .get(name)
                .is_some_and(|entry| entry.life.is_some());
            if !runs {
                outcome.kept.push(name.clone());
                continue;
            }
            match self.reload_one(name, drain).await {
                Ok(_) => outcome.reloaded.push(name.clone()),
                Err(err) => {
                    tracing::error!(target: "tilrd::bus", "reload of {name}: {err}");
                    failure.get_or_insert(BusError::new(
                        err.code.clone(),
                        format!("{name}: {}", err.message),
                    ));
                    failed.push(name.clone());
                }
            }
        }
        outcome.kept.extend(diff.kept.iter().cloned());
        outcome.kept.sort();
        // Added ones start together; each is waited for until its `Hello`.
        let mut waiting = Vec::new();
        for name in &diff.added {
            let autostart = self
                .table()
                .instances
                .get(name)
                .is_some_and(|entry| entry.wanted);
            if !autostart {
                continue;
            }
            match self.begin_start(name, false) {
                Ok(rx) => waiting.push((name.clone(), rx)),
                Err(err) => {
                    failure.get_or_insert(BusError::new(
                        err.code.clone(),
                        format!("{name}: {}", err.message),
                    ));
                    failed.push(name.clone());
                }
            }
        }
        for (name, rx) in waiting {
            match rx.await {
                Ok(Ok(_)) => outcome.started.push(name),
                Ok(Err(err)) => {
                    tracing::error!(target: "tilrd::bus", "start of {name}: {err}");
                    failure.get_or_insert(BusError::new(
                        err.code.clone(),
                        format!("{name}: {}", err.message),
                    ));
                    failed.push(name);
                }
                Err(_) => outcome.started.push(name),
            }
        }
        match failure {
            Some(err) => {
                failed.sort();
                Err(BusError::new(
                    err.code,
                    format!("{}; {}", err.message, applied(&outcome, &failed)),
                ))
            }
            None => Ok(outcome),
        }
    }

    fn with_entry<R>(&self, name: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
        self.table().instances.get_mut(name).map(f)
    }

    /// The spec and module `name` is started with now; `None` once it left `bus.toml`.
    fn specs(&self, name: &str) -> Option<(InstanceSpec, Module)> {
        self.table()
            .instances
            .get(name)
            .map(|entry| (entry.spec.clone(), entry.module.clone()))
    }
}

/// What a `bus.reload_config` that failed applied all the same, for its error message:
/// `bus.toml` is taken (the table holds the new file) whether or not every instance came up.
fn applied(outcome: &ConfigOutcome, failed: &[String]) -> String {
    let list = |names: &[String]| format!("[{}]", names.join(", "));
    format!(
        "bus.toml is applied: stopped {}, reloaded {}, started {}, kept {}; not up: {}",
        list(&outcome.stopped),
        list(&outcome.reloaded),
        list(&outcome.started),
        list(&outcome.kept),
        list(failed),
    )
}

impl Entry {
    fn new(spec: InstanceSpec, module: Module) -> Self {
        Self {
            wanted: spec.autostart,
            spec,
            module,
            life: None,
            snap: Snap::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// The slot on the supervisor, and its hooks
// ---------------------------------------------------------------------------

/// What the supervisor keeps of the runner (`CoreState::slot`): the hooks reach it.
#[derive(Default)]
pub struct RunnerSlot(Option<Runner>);

/// Start the runner with the core: read `bus.toml`, start its instances and keep the
/// handle where the supervisor's hooks find it.
pub fn start(core: &Core) -> Runner {
    // Never the working directory: without a configuration directory there are no instances.
    let config_dir = config::config_dir();
    if config_dir.is_none() {
        tracing::warn!(target: "tilrd::bus", "no configuration directory: no bus instances");
    }
    let runner = Runner::new(
        core.broker.clone(),
        config_dir,
        crate::clipboard::runtime_dir(),
    );
    runner.start_from_file();
    let held = runner.clone();
    let _ = core
        .supervisor
        .with(move |supervisor| supervisor.bus.slot::<RunnerSlot>().0 = Some(held));
    runner
}

fn runner_of(supervisor: &Supervisor) -> Option<Runner> {
    supervisor
        .bus
        .slot_ref::<RunnerSlot>()
        .and_then(|slot| slot.0.clone())
}

/// Hook (`supervisor.rs`): the server exits or is about to exec its successor, which
/// starts the instances again from `bus.toml`: stop them all now.
pub async fn stop_for_exit(supervisor: &Supervisor) {
    if let Some(runner) = runner_of(supervisor) {
        runner.stop_all().await;
    }
}

/// Hook (`supervisor.rs`): the exec did not happen and the server carries on: start
/// the instances `stop_for_exit` stopped.
pub fn resume_after_failed_exit(supervisor: &Supervisor) {
    if let Some(runner) = runner_of(supervisor) {
        runner.resume_all();
    }
}

// ---------------------------------------------------------------------------
// One instance's life
// ---------------------------------------------------------------------------

struct ReloadWait {
    old_pid: u32,
    done: oneshot::Sender<Result<(u32, u32), BusError>>,
}

struct Life {
    runner: Runner,
    name: String,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    waiters: Vec<oneshot::Sender<StartResult>>,
    reload: Option<ReloadWait>,
}

enum Outcome {
    /// The process or its connection ended by itself; `ran` is how long it was up.
    Failed { ran: Duration },
    /// Asked to stop, or gone from `bus.toml`: the task ends.
    Stopped,
    /// Asked to reload: start again at once.
    Reload,
}

enum Paused {
    Elapsed,
    Reload,
    Stopped,
}

impl Life {
    async fn run(mut self) {
        let mut backoff = BACKOFF_MIN;
        let mut restart = false;
        loop {
            if restart {
                self.set(|snap| snap.restarts += 1);
            }
            match self.attempt().await {
                Outcome::Stopped => return,
                Outcome::Reload => {
                    restart = false;
                    backoff = BACKOFF_MIN;
                }
                Outcome::Failed { ran } => {
                    if ran >= STABLE {
                        backoff = BACKOFF_MIN;
                    }
                    self.set(|snap| {
                        snap.state = State::Down;
                        snap.pid = 0;
                        snap.conn = None;
                    });
                    let pause = backoff;
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                    tracing::info!(target: "tilrd::bus", "instance {} starts again in {} ms", self.name, pause.as_millis());
                    match self.pause(pause).await {
                        Paused::Elapsed => restart = true,
                        Paused::Reload => {
                            restart = false;
                            backoff = BACKOFF_MIN;
                        }
                        Paused::Stopped => return,
                    }
                }
            }
        }
    }

    fn set(&self, f: impl FnOnce(&mut Snap)) {
        self.runner
            .with_entry(&self.name, |entry| f(&mut entry.snap));
    }

    /// Draining in the broker before it is told: whoever sees the instance
    /// `draining` sees it offered no new calls.
    async fn draining(&self, conn: ConnId) {
        self.runner.0.broker.set_draining(conn, true);
        // Held here by a test: drained in the broker, not yet told.
        #[cfg(feature = "failpoints")]
        crate::fail::point_async(format!("bus-draining:{}", self.name)).await;
        self.set(|snap| snap.state = State::Draining);
        tracing::info!(target: "tilrd::bus", "instance {} is draining", self.name);
    }

    /// Tell whoever waits how the start went.
    fn resolve(&mut self, result: StartResult) {
        for waiter in self.waiters.drain(..) {
            let _ = waiter.send(result.clone());
        }
        if let Some(ReloadWait { old_pid, done }) = self.reload.take() {
            let _ = done.send(result.map(|new_pid| (old_pid, new_pid)));
        }
    }

    /// The task ends: the instance is stopped (or gone).
    fn finish(&mut self, done: Option<oneshot::Sender<()>>) {
        self.runner.with_entry(&self.name, |entry| {
            entry.life = None;
            entry.snap.state = State::Stopped;
            entry.snap.pid = 0;
            entry.snap.conn = None;
        });
        tracing::info!(target: "tilrd::bus", "instance {} is stopped", self.name);
        self.resolve(Err(BusError::unavailable(format!(
            "{} was stopped",
            self.name
        ))));
        if let Some(done) = done {
            let _ = done.send(());
        }
    }

    /// Wait out the pause before a restart.
    async fn pause(&mut self, pause: Duration) -> Paused {
        let wake = tokio::time::sleep(pause);
        tokio::pin!(wake);
        loop {
            tokio::select! {
                () = &mut wake => return Paused::Elapsed,
                cmd = self.cmds.recv() => match cmd {
                    None => {
                        self.finish(None);
                        return Paused::Stopped;
                    }
                    Some(Cmd::Wait(tx)) => self.waiters.push(tx),
                    Some(Cmd::Stop { done, .. }) => {
                        self.finish(Some(done));
                        return Paused::Stopped;
                    }
                    Some(Cmd::Reload { done, .. }) => {
                        self.reload = Some(ReloadWait { old_pid: 0, done });
                        return Paused::Reload;
                    }
                },
            }
        }
    }

    /// One process, from its start to its end.
    async fn attempt(&mut self) -> Outcome {
        let Some((spec, module)) = self.runner.specs(&self.name) else {
            self.finish(None);
            return Outcome::Stopped;
        };
        // The server is exiting (a reload drained into it, or a pause ended): no new
        // process; `resume_all` starts it again if the exec fails.
        if self.runner.table().exiting {
            self.finish(None);
            return Outcome::Stopped;
        }
        self.set(|snap| {
            snap.state = State::Starting;
            snap.pid = 0;
            snap.conn = None;
        });
        let mut running = match launch(&self.runner, &self.name, &spec, &module) {
            Ok(running) => running,
            Err(error) => {
                tracing::error!(target: "tilrd::bus", "instance {}: {}", self.name, error.message);
                self.resolve(Err(error));
                return Outcome::Failed {
                    ran: Duration::ZERO,
                };
            }
        };
        let pid = running.pid;
        // Each change of the listing is logged after it is made, so a reader of the log
        // finds it in `bus.instances` (the tests wait on these lines).
        self.set(|snap| snap.pid = pid);
        tracing::info!(target: "tilrd::bus", "instance {} started (pid {pid})", self.name);
        let started = Instant::now();

        // Until its Hello.
        let limit = tokio::time::sleep(process_hello_limit());
        tokio::pin!(limit);
        let mut conn = loop {
            tokio::select! {
                bound = running.bound.recv() => match bound {
                    Some(Bound::Up(conn)) => break conn,
                    Some(Bound::Ended(ended)) => {
                        let why = match &ended.refusal {
                            Some(refusal) => format!("refused: {}: {}", refusal.code, refusal.message),
                            None => format!("its connection ended before it said Hello: {}", ended.reason),
                        };
                        tracing::error!(target: "tilrd::bus", "instance {}: {why}", self.name);
                        terminate(&mut running).await;
                        self.resolve(Err(BusError::new(
                            codes::START_FAILED,
                            format!("{} did not come up: {why}", self.name),
                        )));
                        return Outcome::Failed { ran: started.elapsed() };
                    }
                    None => std::future::pending::<()>().await,
                },
                () = exec::exited(pid, &mut running.sigchld) => {
                    let status = reap(&mut running).await;
                    let how = describe(&status);
                    tracing::error!(target: "tilrd::bus", "instance {} {how} before its Hello", self.name);
                    self.resolve(Err(BusError::new(
                        codes::START_FAILED,
                        format!("{} {how} before it said Hello", self.name),
                    )));
                    return Outcome::Failed { ran: started.elapsed() };
                }
                () = &mut limit => {
                    tracing::error!(target: "tilrd::bus", "instance {} said no Hello in time", self.name);
                    terminate(&mut running).await;
                    self.resolve(Err(BusError::new(
                        codes::START_TIMEOUT,
                        format!("{} said no Hello in time", self.name),
                    )));
                    return Outcome::Failed { ran: started.elapsed() };
                }
                cmd = self.cmds.recv() => match cmd {
                    None => {
                        terminate(&mut running).await;
                        self.finish(None);
                        return Outcome::Stopped;
                    }
                    Some(Cmd::Wait(tx)) => self.waiters.push(tx),
                    Some(Cmd::Stop { done, .. }) => {
                        terminate(&mut running).await;
                        self.finish(Some(done));
                        return Outcome::Stopped;
                    }
                    Some(Cmd::Reload { done, .. }) => {
                        terminate(&mut running).await;
                        self.reload = Some(ReloadWait { old_pid: pid, done });
                        return Outcome::Reload;
                    }
                },
            }
        };

        // Up. What it provides stays known to the broker once it is gone: a call of it
        // while the instance is down or stopped is `unavailable`, not `not_found`.
        let provides = self
            .runner
            .0
            .broker
            .connections()
            .into_iter()
            .find(|info| info.connection_id == conn.id)
            .map(|info| info.provides)
            .unwrap_or_default();
        self.runner
            .0
            .broker
            .know_methods(provides.into_iter().map(|spec| spec.method));
        self.set(|snap| {
            snap.state = State::Up;
            snap.conn = Some(conn.id);
        });
        tracing::info!(target: "tilrd::bus", "instance {} is up (pid {pid})", self.name);
        self.resolve(Ok(pid));
        let up = Instant::now();
        let mut state = conn.state.clone();
        loop {
            tokio::select! {
                () = exec::exited(pid, &mut running.sigchld) => {
                    let status = reap(&mut running).await;
                    tracing::error!(target: "tilrd::bus", "instance {} {}", self.name, describe(&status));
                    conn.close(&self.runner.0.broker);
                    return Outcome::Failed { ran: up.elapsed() };
                }
                changed = state.changed() => {
                    let ended = match (&changed, &*state.borrow()) {
                        (_, ConnState::Ended(ended)) => Some(ended.reason.clone()),
                        (Err(_), _) => Some("the broker dropped it".to_owned()),
                        _ => None,
                    };
                    if let Some(reason) = ended {
                        tracing::error!(target: "tilrd::bus", "instance {}: its connection ended: {reason}", self.name);
                        terminate(&mut running).await;
                        conn.close(&self.runner.0.broker);
                        return Outcome::Failed { ran: up.elapsed() };
                    }
                }
                cmd = self.cmds.recv() => match cmd {
                    None => {
                        wind_down(&self.runner, &mut running, &mut conn, Duration::ZERO).await;
                        self.finish(None);
                        return Outcome::Stopped;
                    }
                    Some(Cmd::Wait(tx)) => {
                        let _ = tx.send(Ok(pid));
                    }
                    Some(Cmd::Stop { drain, done }) => {
                        self.draining(conn.id).await;
                        wind_down(&self.runner, &mut running, &mut conn, drain).await;
                        self.finish(Some(done));
                        return Outcome::Stopped;
                    }
                    Some(Cmd::Reload { drain, done }) => {
                        self.draining(conn.id).await;
                        wind_down(&self.runner, &mut running, &mut conn, drain).await;
                        self.reload = Some(ReloadWait { old_pid: pid, done });
                        return Outcome::Reload;
                    }
                },
            }
        }
    }
}

fn describe(status: &std::io::Result<std::process::ExitStatus>) -> String {
    match status {
        Ok(status) => format!("exited ({status})"),
        Err(err) => format!("could not be waited for: {err}"),
    }
}

/// Drain the connection, stop the process and close the connection. Once the server
/// exits ([`Runner::stop_all`]) the drain ends at once, however long it was given: the
/// calls still in flight are cancelled.
async fn wind_down(runner: &Runner, running: &mut Running, conn: &mut Conn, drain: Duration) {
    let broker = &runner.0.broker;
    tokio::select! {
        _ = broker.drain_connection(conn.id, drain) => {}
        () = exiting(runner) => {
            broker.drain_connection(conn.id, Duration::ZERO).await;
        }
    }
    terminate(running).await;
    conn.close(broker);
}

/// Once the server exits or execs.
async fn exiting(runner: &Runner) {
    let mut exit = runner.0.exit.subscribe();
    let _ = exit.wait_for(|exiting| *exiting).await;
}

/// SIGTERM to the process group, SIGKILL after [`KILL_AFTER`] or once the leader is
/// gone, then reap the leader. The group is signalled only while the leader is not
/// reaped (a zombie keeps its pid, and so the group's id, from being reused).
async fn terminate(running: &mut Running) {
    if running.child.id().is_none() {
        return;
    }
    let pid = running.pid;
    kill_group(pid, libc::SIGTERM);
    let _ = tokio::time::timeout(KILL_AFTER, exec::exited(pid, &mut running.sigchld)).await;
    kill_group(pid, libc::SIGKILL);
    let _ = running.child.wait().await;
}

/// The leader has exited and is not reaped yet: kill what its group still holds (the
/// zombie keeps the group's id from being reused), then reap it.
async fn reap(running: &mut Running) -> std::io::Result<std::process::ExitStatus> {
    kill_group(running.pid, libc::SIGKILL);
    running.child.wait().await
}

fn kill_group(pid: u32, signal: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if pid <= 1 {
        return;
    }
    // SAFETY: killpg has no memory-safety preconditions; the group is the one of the
    // process this runner started in a session of its own (setsid), so `pid` is its id.
    unsafe {
        libc::killpg(pid, signal);
    }
}

// ---------------------------------------------------------------------------
// Starting a process
// ---------------------------------------------------------------------------

/// What a watched connection tells its lifecycle.
enum Bound {
    /// Greeted: this is the instance.
    Up(Conn),
    /// Ended before it was greeted (refused, or gone).
    Ended(Ended),
}

/// The instance's greeted connection.
struct Conn {
    id: ConnId,
    state: watch::Receiver<ConnState>,
    shutter: Shutter,
}

impl Conn {
    fn close(&mut self, broker: &BrokerHandle) {
        broker.close_connection(self.id, 1000, "instance stopped");
        self.shutter.shutdown();
    }
}

/// A second descriptor of a connection's socket, to shut it down from here: a
/// connection not greeted yet is unknown to the broker, and a stuck one must not hold
/// the instance's end open.
struct Shutter(Option<OwnedFd>);

impl Shutter {
    fn shutdown(&mut self) {
        if let Some(fd) = self.0.take() {
            // SAFETY: shutdown on a descriptor this struct owns; the result does not matter.
            unsafe {
                libc::shutdown(fd.as_raw_fd(), libc::SHUT_RDWR);
            }
        }
    }
}

/// A started process and what belongs to it while it runs.
struct Running {
    child: Child,
    pid: u32,
    /// Wakes the wait for the leader's exit ([`exec::exited`]).
    sigchld: Option<Signal>,
    bound: mpsc::UnboundedReceiver<Bound>,
    /// Keeps `bound` open while nothing else sends on it.
    _bound_tx: mpsc::UnboundedSender<Bound>,
    accept: Option<JoinHandle<()>>,
    socket_file: Option<PathBuf>,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(accept) = self.accept.take() {
            accept.abort();
        }
        if let Some(path) = self.socket_file.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn spawn_failed(name: &str, what: impl std::fmt::Display) -> BusError {
    BusError::new(codes::SPAWN_FAILED, format!("{name}: {what}"))
}

/// The server's environment for an instance: no pane variables, no bus variables.
fn base_env() -> Vec<(OsString, OsString)> {
    let pane = pane_variables();
    std::env::vars_os()
        .filter(|(name, _)| {
            let name = name.to_string_lossy();
            !pane.contains(&&*name) && !name.starts_with("TILR_BUS_")
        })
        .collect()
}

/// The descriptors above 2 the server has open now, to close in the child.
fn open_descriptors() -> Vec<RawFd> {
    let Ok(dir) = std::fs::read_dir("/dev/fd") else {
        return Vec::new();
    };
    dir.filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<RawFd>().ok())
        .filter(|fd| *fd > 2)
        .collect()
}

fn random_token() -> Result<String, BusError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|err| BusError::new(codes::SPAWN_FAILED, format!("no random numbers: {err}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn bind_socket(path: &Path) -> Result<UnixListener, BusError> {
    use std::os::unix::fs::PermissionsExt;
    let parent = path
        .parent()
        .ok_or_else(|| spawn_failed(&path.display().to_string(), "no directory"))?;
    crate::identity::create_private_dir(parent)
        .map_err(|err| spawn_failed(&parent.display().to_string(), format!("{err:#}")))?;
    let _ = std::fs::remove_file(path);
    let listener =
        UnixListener::bind(path).map_err(|err| spawn_failed(&path.display().to_string(), err))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| spawn_failed(&path.display().to_string(), err))?;
    Ok(listener)
}

/// Start the process of `name` and attach its bus connection.
fn launch(
    runner: &Runner,
    name: &str,
    spec: &InstanceSpec,
    module: &Module,
) -> Result<Running, BusError> {
    let Some(config_dir) = &runner.0.config_dir else {
        return Err(spawn_failed(name, "no configuration directory"));
    };
    let program = &module.command[0];
    let mut command = Command::new(program);
    command.args(&module.command[1..]);
    command.current_dir(config::working_dir(module, config_dir));
    command.env_clear();
    command.envs(base_env());
    command.envs(&module.env);
    command.envs(&spec.env);
    command.env("TILR_INSTANCE", name);
    command.env("TILR_CONFIG_DIR", config_dir);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let (bound_tx, bound) = mpsc::unbounded_channel();
    let mut parent_fd: Option<OwnedFd> = None;
    let mut child_fd: Option<OwnedFd> = None;
    let mut listener = None;
    let mut token = String::new();
    let mut socket_file = None;
    match spec.connect {
        Connect::Fd => {
            let (parent, child) = std::os::unix::net::UnixStream::pair()
                .map_err(|err| spawn_failed(name, format!("socket pair: {err}")))?;
            command.env(bus_env::BUS_FD, "3");
            parent_fd = Some(parent.into());
            child_fd = Some(child.into());
        }
        Connect::Socket => {
            let path = config::socket_path(&runner.0.runtime_dir, name);
            listener = Some(bind_socket(&path)?);
            socket_file = Some(path.clone());
            token = random_token()?;
            command.env(bus_env::BUS_SOCK, &path);
            command.env(bus_env::BUS_TOKEN, &token);
        }
    }

    let child_raw = child_fd.as_ref().map(AsRawFd::as_raw_fd);
    // Before the spawn, so a SIGCHLD of this process is not missed.
    let sigchld = signal(SignalKind::child()).ok();
    let close = open_descriptors();
    // SAFETY: the closure runs between fork and exec and calls only async-signal-safe
    // functions (setsid, dup2, fcntl, close) on descriptors it owns in the child; it
    // allocates nothing.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if let Some(fd) = child_raw {
                if fd == 3 {
                    if libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else if libc::dup2(fd, 3) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // The list was taken before the fork, so a number in it may be another
            // descriptor by now (std's pipe for the exec error, opened later, must stay
            // open until the exec). Only a descriptor the exec would keep is closed: one
            // with FD_CLOEXEC goes at the exec anyway.
            for fd in &close {
                if child_raw.is_some() && *fd == 3 {
                    continue;
                }
                let flags = libc::fcntl(*fd, libc::F_GETFD);
                if flags != -1 && flags & libc::FD_CLOEXEC == 0 {
                    libc::close(*fd);
                }
            }
            Ok(())
        });
    }
    // With SIGTERM and the other signals the supervisor's threads hold let in: a child
    // inherits the mask, and one started with SIGTERM blocked is only ended by SIGKILL.
    let mut child = crate::reload::spawn_unblocked(|| command.spawn())
        .map_err(|err| spawn_failed(name, format!("{program}: {err}")))?;
    drop(child_fd);
    let Some(pid) = child.id() else {
        return Err(spawn_failed(name, "the process is gone already"));
    };
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pump(name.to_owned(), stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(pump(name.to_owned(), stderr));
    }

    let rights = Arc::new(spec.rights.clone());
    let identity = Identity::Instance(name.to_owned());
    let peer = Peer::Child { pid: Some(pid) };
    let mut accept = None;
    if let Some(fd) = parent_fd {
        let shutter = Shutter(fd.try_clone().ok());
        let attached = runner.0.broker.attach_stream(
            fd,
            Seat {
                identity,
                rights: Some(rights),
                transport: Transport::InstanceFd,
                peer,
                check: None,
            },
        );
        tokio::spawn(watch_conn(attached, shutter, bound_tx.clone(), None, None));
    } else if let Some(listener) = listener {
        accept = Some(tokio::spawn(accept_loop(
            listener,
            runner.0.broker.clone(),
            identity,
            rights,
            peer,
            token,
            bound_tx.clone(),
        )));
    }
    Ok(Running {
        child,
        pid,
        sigchld,
        bound,
        _bound_tx: bound_tx,
        accept,
        socket_file,
    })
}

/// Serve an instance socket: every connection is attached with a check of the one-time
/// token; the first with the token is the instance, any other is refused and closed.
async fn accept_loop(
    listener: UnixListener,
    broker: BrokerHandle,
    identity: Identity,
    rights: Arc<super::identity::Rights>,
    peer: Peer,
    token: String,
    bound: mpsc::UnboundedSender<Bound>,
) {
    let token: Arc<str> = Arc::from(token);
    let consumed = Arc::new(AtomicBool::new(false));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(err) => {
                tracing::warn!(target: "tilrd::bus", "instance socket: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let fd: OwnedFd = match stream.into_std() {
            Ok(stream) => stream.into(),
            Err(err) => {
                tracing::warn!(target: "tilrd::bus", "instance socket: {err}");
                continue;
            }
        };
        let shutter = Shutter(fd.try_clone().ok());
        let mine = Arc::new(AtomicBool::new(false));
        let check = {
            let token = Arc::clone(&token);
            let consumed = Arc::clone(&consumed);
            let mine = Arc::clone(&mine);
            Box::new(move |hello: &Hello| -> Result<(), ConnectionError> {
                let denied = |message: &str| ConnectionError {
                    code: codes::DENIED.into(),
                    message: message.into(),
                };
                if !crate::auth::matches(&token, &hello.instance_token) {
                    return Err(denied("this instance socket takes its instance's token"));
                }
                if consumed.swap(true, Ordering::SeqCst) {
                    return Err(denied("the instance token was used already"));
                }
                mine.store(true, Ordering::SeqCst);
                Ok(())
            })
        };
        let attached = broker.attach_stream(
            fd,
            Seat {
                identity: identity.clone(),
                rights: Some(Arc::clone(&rights)),
                transport: Transport::InstanceSocket,
                peer,
                check: Some(check),
            },
        );
        tokio::spawn(watch_conn(
            attached,
            shutter,
            bound.clone(),
            Some(socket_hello_limit()),
            Some(mine),
        ));
    }
}

/// Follow a connection until it is greeted (tell the lifecycle: it is the instance) or
/// ends; shut it down if it says no `Hello` within `limit`. A connection that is not
/// `mine` (a stranger on an instance socket) is closed quietly; the lifecycle hears of
/// the one that is.
async fn watch_conn(
    attached: Attached,
    mut shutter: Shutter,
    bound: mpsc::UnboundedSender<Bound>,
    limit: Option<Duration>,
    mine: Option<Arc<AtomicBool>>,
) {
    let Attached {
        connection_id,
        mut state,
    } = attached;
    let deadline = limit.map(|limit| tokio::time::Instant::now() + limit);
    let is_mine = || mine.as_ref().is_none_or(|mine| mine.load(Ordering::SeqCst));
    loop {
        let current = state.borrow_and_update().clone();
        match current {
            ConnState::Up => {
                let conn = Conn {
                    id: connection_id,
                    state,
                    shutter,
                };
                if let Err(mpsc::error::SendError(Bound::Up(mut conn))) =
                    bound.send(Bound::Up(conn))
                {
                    conn.shutter.shutdown();
                }
                return;
            }
            ConnState::Ended(ended) => {
                if is_mine() {
                    let _ = bound.send(Bound::Ended(ended));
                }
                // Not shut down: the broker is still flushing a `ConnectionError` to the
                // peer; dropping the extra descriptor lets its close reach the peer.
                drop(shutter);
                return;
            }
            // Waiting for its Hello, or a state of a later change: wait for the next.
            _ => {}
        }
        let late = tokio::select! {
            changed = state.changed() => changed.is_err(),
            () = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                tracing::info!(target: "tilrd::bus", connection = connection_id, "no Hello on an instance socket in time: closed");
                shutter.shutdown();
                return;
            }
        };
        if late {
            drop(shutter);
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// An instance's output into the log, a line at a time.
async fn pump<R: AsyncRead + Unpin>(name: String, stream: R) {
    read_lines(stream, LINE_MAX, |line| {
        tracing::info!(target: "tilrd::bus", "bus {name}: {line}");
    })
    .await;
}

/// Read `stream` to its end and hand each line to `emit`; a line over `max` bytes is cut
/// there (the rest of it is dropped). A last line without a newline is a line.
async fn read_lines<R: AsyncRead + Unpin>(mut stream: R, max: usize, mut emit: impl FnMut(&str)) {
    let mut line: Vec<u8> = Vec::new();
    let mut dropping = false;
    let mut chunk = vec![0u8; 8192];
    loop {
        let read = match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for &byte in &chunk[..read] {
            if byte == b'\n' {
                if !dropping {
                    emit(&String::from_utf8_lossy(&line));
                }
                line.clear();
                dropping = false;
            } else if !dropping {
                if line.len() == max {
                    emit(&String::from_utf8_lossy(&line));
                    line.clear();
                    dropping = true;
                } else {
                    line.push(byte);
                }
            }
        }
    }
    if !line.is_empty() && !dropping {
        emit(&String::from_utf8_lossy(&line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn lines_of(input: Vec<u8>, max: usize) -> Vec<String> {
        let mut got = Vec::new();
        read_lines(&input[..], max, |line| got.push(line.to_owned())).await;
        got
    }

    #[tokio::test]
    async fn an_instances_lines_are_cut_at_the_limit() {
        assert_eq!(
            lines_of(b"one\ntwo\r\nlast".to_vec(), 64).await,
            ["one", "two\r", "last"]
        );
        // Over the limit: the first `max` bytes are the line, the rest of it is dropped,
        // and the next line is whole.
        let mut input = vec![b'x'; 100];
        input.extend_from_slice(b"\nnext\n");
        let got = lines_of(input, 64).await;
        assert_eq!(got, ["x".repeat(64), "next".to_owned()]);
        // Exactly the limit is not cut.
        let mut input = vec![b'y'; 64];
        input.push(b'\n');
        assert_eq!(lines_of(input, 64).await, ["y".repeat(64)]);
        assert!(lines_of(Vec::new(), 64).await.is_empty());
    }

    #[test]
    fn states_are_named_as_listed() {
        let names: Vec<_> = [
            State::Down,
            State::Starting,
            State::Up,
            State::Draining,
            State::Stopped,
        ]
        .map(State::as_str)
        .into();
        assert_eq!(names, ["down", "starting", "up", "draining", "stopped"]);
    }
}
