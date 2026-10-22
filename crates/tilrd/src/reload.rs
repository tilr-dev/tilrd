//! Picking up a new executable without closing any shell.
//!
//! The supervisor replaces itself with the executable now on disk, the same
//! way a pane process does: it execs in place, so its pid stays and the pane
//! processes stay its children. The listening socket and the pipes to the
//! pane processes carry over as open descriptors; what the supervisor knows
//! about the panes travels in a file the successor finds by its pid. The new
//! supervisor then asks every pane process to reload too.
//!
//! Nothing about a reload goes on the command line: a process is exec'd with
//! the arguments it was started with, and looks the same in `ps` afterwards.
//!
//! The state files have a format version. A supervisor asks the executable on
//! disk which versions it reads before handing over to it: once it has
//! exec'd there is no way back, and a successor that misreads the state
//! would lose every shell.
//!
//! Then it has the executable show that it can resume: it runs it once more,
//! as a child, with the command line and environment the successor will
//! have and [`CHECK_ENV`] naming a copy of the state. That child parses the
//! command line, loads the configuration, reads the state and goes through
//! the resume up to where it would take over the descriptors, answers `ok`
//! or `error: <reason>` on its stdout and exits. Only on `ok` does the
//! process exec. A pane process checks its successor the same way.
//!
//! Client connections do not survive: clients reconnect and find their panes
//! as they left them.

use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Output, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use prost::Message;
use tilr_proto::SupervisorState;
use uuid::Uuid;

use crate::clipboard;

/// How often the executable on disk is checked for a change.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
/// How long `state-formats` and the check of a successor may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Compatibility tests await their verdict rather than race the host's
/// process scheduling. Release builds always enforce the production limit.
fn state_formats_timeout() -> Option<Duration> {
    #[cfg(feature = "failpoints")]
    if std::env::var_os("TILR_TEST_STATE_FORMATS_UNBOUNDED").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        return None;
    }
    Some(CHECK_TIMEOUT)
}
/// Set for a process that only checks it could resume: the path of the copy
/// of the state it reads.
pub const CHECK_ENV: &str = "TILR_RELOAD_CHECK";
/// How old a state file must be to count as left behind.
const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

/// Identity of a file's current content, as far as a stat can tell.
fn signature(path: &Path) -> Option<(u64, u64, i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.ino(), meta.size(), meta.mtime(), meta.mtime_nsec()))
}

/// Call `changed` whenever the executable at `exe` is replaced. A change
/// counts once the file has looked the same on two checks in a row, so a
/// build still writing it is not picked up half-done.
pub async fn watch_executable(exe: &Path, mut changed: impl FnMut() -> bool) {
    let mut running = signature(exe);
    let mut seen = running;
    let mut ticks = tokio::time::interval(WATCH_INTERVAL);
    loop {
        ticks.tick().await;
        let now = signature(exe);
        if now.is_some() && now == seen && now != running {
            running = now;
            if !changed() {
                return;
            }
        }
        seen = now;
    }
}

/// Version of what a supervisor leaves its successor ([`SupervisorState`]).
/// Raise it with any change an older or newer supervisor would misread.
pub const STATE_FORMAT: u32 = 2;

/// What `tilrd state-formats` prints: the versions of the state files
/// this executable reads, one `name version` per line.
pub fn formats() -> String {
    format!(
        "{SUPERVISOR_STATE} {STATE_FORMAT}\n{PANE_STATE} {}\n{UNIX_SOCKET} 1\n{RELOAD_CHECK} 1\n",
        crate::pane::STATE_FORMAT
    )
}

const SUPERVISOR_STATE: &str = "supervisor-state";
const PANE_STATE: &str = "pane-state";
/// Said by an executable that takes over the Unix socket for local clients
/// (`SupervisorState.local_*`). One that does not say it would ignore those
/// fields and leave the socket file with nobody serving it: a server that
/// serves its socket does not reload into it.
const UNIX_SOCKET: &str = "unix-socket";
/// Said by an executable that answers the check of a successor
/// ([`CHECK_ENV`]). One that does not say it is not reloaded into: started
/// for the check, it would run as a server or a pane of its own.
const RELOAD_CHECK: &str = "reload-check";

/// Whether state written as version `written` can be read as version
/// `read`. Version 0 is state from before versions were recorded, which has
/// the layout of version 1.
pub fn format_matches(written: u32, read: u32) -> bool {
    written.max(1) == read
}

/// Check what another executable printed for `state-formats` against the
/// state this one writes. `Ok(true)` if it also takes over the Unix socket.
fn check_formats(printed: &str) -> Result<bool> {
    for (name, written) in [
        (SUPERVISOR_STATE, STATE_FORMAT),
        (PANE_STATE, crate::pane::STATE_FORMAT),
    ] {
        let read: u32 = printed
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .with_context(|| format!("it does not say which {name} format it reads"))?
            .trim()
            .parse()
            .with_context(|| format!("{name} format"))?;
        if !format_matches(written, read) {
            bail!("it reads {name} format {read}, this server writes {written}");
        }
    }
    let says = |name: &str| {
        printed
            .lines()
            .filter_map(|line| line.strip_prefix(name))
            .any(|version| version.trim() == "1")
    };
    if !says(RELOAD_CHECK) {
        bail!("it cannot show that it can resume (no `{RELOAD_CHECK} 1`)");
    }
    Ok(says(UNIX_SOCKET))
}

/// Make sure the executable on disk runs and reads the state this server
/// would leave it, before betting every pane on it. `Ok(true)` if it also
/// takes over the Unix socket.
pub async fn check_executable(exe: &Path) -> Result<bool> {
    let mut command = tokio::process::Command::new(exe);
    command
        .arg("state-formats")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = async {
        spawn_unblocked(|| command.spawn())?
            .wait_with_output()
            .await
    };
    let result = match state_formats_timeout() {
        Some(limit) => tokio::time::timeout(limit, output).await,
        None => Ok(output.await),
    };
    match result {
        Ok(Ok(output)) if output.status.success() => {
            check_formats(&String::from_utf8_lossy(&output.stdout))
                .with_context(|| format!("{} cannot take over", exe.display()))
        }
        Ok(Ok(output)) => bail!("{} state-formats: {}", exe.display(), output.status),
        Ok(Err(err)) => Err(err).with_context(|| format!("run {}", exe.display())),
        Err(_) => bail!("{} state-formats did not finish", exe.display()),
    }
}

/// Have the executable at `exe` show that it can resume from `state`: run it
/// with `args` (those it will be exec'd with) and [`CHECK_ENV`] naming a copy
/// of the state at `check_file`, and wait for its `ok`. The copy is removed
/// whatever happens, and a child that does not answer in time is killed.
pub async fn check_successor(
    exe: &Path,
    args: Vec<OsString>,
    check_file: PathBuf,
    state: &[u8],
) -> Result<()> {
    write_file(&check_file, state).context("write the state to check")?;
    let check_file = Removed(check_file);
    let mut command = tokio::process::Command::new(exe);
    command
        .args(args)
        .env(CHECK_ENV, &check_file.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child =
        spawn_unblocked(|| command.spawn()).with_context(|| format!("run {}", exe.display()))?;
    match tokio::time::timeout(CHECK_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => verdict(&output),
        Ok(Err(err)) => Err(err).with_context(|| format!("run {}", exe.display())),
        Err(_) => bail!("it did not answer within {} s", CHECK_TIMEOUT.as_secs()),
    }
}

/// A file removed when this is dropped.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What a check child's answer and exit say.
fn verdict(output: &Output) -> Result<()> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let answer = stdout.lines().next().unwrap_or_default().trim();
    if answer == "ok" {
        if output.status.success() {
            return Ok(());
        }
        bail!("it answered ok but ended with {}", output.status);
    }
    if let Some(reason) = answer.strip_prefix("error: ") {
        bail!("{reason}");
    }
    // It died before answering: a crash, or something that is not tilrd.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut said: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    said.drain(..said.len().saturating_sub(3));
    let said = said.join(" | ");
    let said: String = said.chars().take(400).collect();
    if said.is_empty() {
        bail!("it ended with {} without answering", output.status);
    }
    bail!("it ended with {} without answering: {said}", output.status)
}

/// Run `check` as the check child of a reload: answer `ok` or `error:
/// <reason>` on stdout, one line, and exit 0 or 1. A panic is answered too.
pub fn answer_check(check: impl FnOnce() -> Result<()>) -> ExitCode {
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let at = info
            .location()
            .map(|at| format!(" at {}:{}", at.file(), at.line()))
            .unwrap_or_default();
        answer(&format!("error: panicked{at}: {message}"));
    }));
    match check() {
        Ok(()) => {
            answer("ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            answer(&format!("error: {err:#}"));
            ExitCode::FAILURE
        }
    }
}

fn answer(line: &str) {
    let line = line.replace('\n', "; ");
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

/// The signals held across a supervisor's exec, and recorded while a server
/// starts: what arrives before the server's handlers would otherwise end it.
const HELD: [libc::c_int; 4] = [libc::SIGUSR1, libc::SIGHUP, libc::SIGTERM, libc::SIGINT];

fn held_set() -> libc::sigset_t {
    // SAFETY: sigemptyset initializes the set; sigaddset takes valid
    // signal numbers.
    unsafe {
        let mut set = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in HELD {
            libc::sigaddset(&mut set, signal);
        }
        set
    }
}

/// Have `command`, about to be exec'd in place, start with [`HELD`]
/// blocked: they wait, pending, for the successor ([`take_held_signals`]).
/// Done in `pre_exec`, after whatever the standard library does to the mask.
/// The other threads must not take them meanwhile ([`hold_everywhere`]).
pub fn hold_signals(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: pthread_sigmask is async-signal-safe, and the set is built
    // before.
    unsafe {
        let set = held_set();
        command.pre_exec(move || {
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            Ok(())
        });
    }
}

/// Unblock [`HELD`] on this thread.
fn release_held_signals() {
    let set = held_set();
    // SAFETY: the set is valid.
    unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) };
}

/// Start a child with none of [`HELD`] blocked: `spawn` runs with them let
/// in on this thread, whose mask a child inherits. The supervisor's threads
/// have them blocked ([`serve_signals`]); a reload's exec waits until no
/// thread is in here ([`hold_everywhere`]).
pub fn spawn_unblocked<T>(spawn: impl FnOnce() -> T) -> T {
    let Some(thread) = SIGNAL_THREAD.get() else {
        return spawn();
    };
    let mut state = thread.lock();
    while state.holding {
        state = thread.wait(state);
    }
    state.spawning += 1;
    drop(state);
    // From here on whatever happens, a panic included, puts the mask and
    // the count back.
    let _spawning = Spawning(thread);
    release_held_signals();
    // A test makes the start of a child panic here.
    let _ = crate::fail::point("spawn-unblocked");
    spawn()
}

/// A thread in [`spawn_unblocked`]: dropped, it blocks [`HELD`] on the
/// thread again and only then leaves the count, so that the count never
/// says no thread can take them while this one still can. It never panics:
/// a panic while unwinding from another would abort the process.
struct Spawning(&'static SignalThread);

impl Drop for Spawning {
    fn drop(&mut self) {
        let set = held_set();
        // SAFETY: the set is valid.
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
        let mut state = self.0.lock();
        state.spawning -= 1;
        self.0.changed.notify_all();
    }
}

/// The thread [`HELD`] are taken on, and what the others ask of it.
struct SignalThread {
    state: std::sync::Mutex<SignalState>,
    changed: std::sync::Condvar,
}

struct SignalState {
    /// Whether the signal thread is to have them blocked.
    block: bool,
    /// Whether it has.
    blocked: bool,
    /// An exec is under way: no child is started meanwhile.
    holding: bool,
    /// How many threads are starting a child, with them let in.
    spawning: usize,
}

impl SignalThread {
    fn lock(&self) -> std::sync::MutexGuard<'_, SignalState> {
        self.state.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn wait<'a>(
        &self,
        state: std::sync::MutexGuard<'a, SignalState>,
    ) -> std::sync::MutexGuard<'a, SignalState> {
        self.changed
            .wait(state)
            .unwrap_or_else(|err| err.into_inner())
    }

    fn wait_timeout<'a>(
        &self,
        state: std::sync::MutexGuard<'a, SignalState>,
        timeout: Duration,
    ) -> std::sync::MutexGuard<'a, SignalState> {
        self.changed
            .wait_timeout(state, timeout)
            .unwrap_or_else(|err| err.into_inner())
            .0
    }
}

/// How long a reload waits for the signal thread to block [`HELD`] and for
/// the children being started to be started, before it gives up.
const HOLD_TIMEOUT: Duration = Duration::from_secs(5);

static SIGNAL_THREAD: std::sync::OnceLock<SignalThread> = std::sync::OnceLock::new();

/// For a server, before it starts any other thread: [`HELD`] stay blocked
/// on this thread, and so on every thread started from it, all but one: a
/// thread of their own, the only one their handlers run on. Before the
/// supervisor execs a successor that thread blocks them too
/// ([`hold_everywhere`]), so that what comes during the exec waits, pending,
/// for the successor: a signal the system hands to another thread while
/// the exec replaces the image ends the process (macOS).
pub fn serve_signals() -> std::io::Result<()> {
    let set = held_set();
    // SAFETY: the set is valid.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    let thread = SIGNAL_THREAD.get_or_init(|| SignalThread {
        // It starts with them blocked, as this thread has them.
        state: std::sync::Mutex::new(SignalState {
            block: false,
            blocked: true,
            holding: false,
            spawning: 0,
        }),
        changed: std::sync::Condvar::new(),
    });
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let mut state = thread.lock();
            loop {
                if state.block != state.blocked {
                    let how = if state.block {
                        libc::SIG_BLOCK
                    } else {
                        libc::SIG_UNBLOCK
                    };
                    // SAFETY: the set is valid.
                    unsafe { libc::pthread_sigmask(how, &set, std::ptr::null_mut()) };
                    state.blocked = state.block;
                    thread.changed.notify_all();
                }
                state = thread.wait(state);
            }
        })?;
    // Let them in on the signal thread now.
    if !set_signal_thread(thread, false, false, HOLD_TIMEOUT) {
        return Err(std::io::Error::other("the signal thread does not answer"));
    }
    Ok(())
}

/// Before an exec in place: have no thread of this process take [`HELD`],
/// so that what comes waits for the successor. Undone by
/// [`release_everywhere`], also when this fails: after [`HOLD_TIMEOUT`],
/// rather than freeze the thread that asked (the event loop).
pub fn hold_everywhere() -> Result<()> {
    hold_within(HOLD_TIMEOUT)
}

fn hold_within(timeout: Duration) -> Result<()> {
    let Some(thread) = SIGNAL_THREAD.get() else {
        return Ok(());
    };
    if !set_signal_thread(thread, true, true, timeout) {
        let state = thread.lock();
        bail!(
            "the signals cannot be held after {timeout:?}: the signal thread has them {}, \
             {} thread(s) starting a child",
            if state.blocked { "blocked" } else { "let in" },
            state.spawning
        );
    }
    Ok(())
}

/// Let the signal thread take [`HELD`] again, and children be started.
pub fn release_everywhere() {
    if let Some(thread) = SIGNAL_THREAD.get()
        && !set_signal_thread(thread, false, false, HOLD_TIMEOUT)
    {
        tracing::error!("the signal thread does not let the signals in again");
    }
}

/// Have the signal thread block [`HELD`] or let them in and, if `holding`,
/// wait until no thread is starting a child. `false` if that has not
/// happened within `timeout`.
fn set_signal_thread(thread: &SignalThread, block: bool, holding: bool, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let mut state = thread.lock();
    state.holding = holding;
    state.block = block;
    thread.changed.notify_all();
    while state.blocked != block || (holding && state.spawning > 0) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        state = thread.wait_timeout(state, left);
    }
    true
}

/// Set SIGPIPE back to ignored, as the Rust runtime leaves it at start,
/// after an exec in place that failed: the standard library sets it to the
/// default action for the new image before `execvp` and does not undo that
/// when `execvp` fails, after which a write to a pipe whose reader is gone
/// (stderr, a pane process's pipes) would kill this process instead of
/// failing.
pub fn ignore_sigpipe() {
    // SAFETY: setting a disposition to SIG_IGN has no preconditions.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
}

/// While a reload gets ready for its exec: note a SIGTERM or SIGINT, as
/// well as act on it as usual. Between the supervisor's last read of its
/// events and the exec, the event a stop turns into is never read, and the
/// exec would lose it; [`raise_stop`] keeps it for the successor. Armed
/// with the reload, before its last read of events, so that a stop on its
/// way to the event queue then is caught by one or the other. Ended by
/// [`disarm_stop`].
pub fn arm_stop() {
    use std::sync::atomic::Ordering;
    for came in &HELD_CAME {
        came.store(false, Ordering::Relaxed);
    }
    HELD_TAKEN.store(false, Ordering::Relaxed);
}

/// Once [`hold_everywhere`] holds the signals, right before the exec: send
/// this process again the stop [`arm_stop`] noted. No thread takes it now,
/// so it waits, pending, across the exec, and the successor acts on it
/// ([`take_held_signals`]). Nothing is noted after this.
pub fn raise_stop() {
    use std::sync::atomic::Ordering;
    HELD_TAKEN.store(true, Ordering::Relaxed);
    for (at, signal) in [(2, libc::SIGTERM), (3, libc::SIGINT)] {
        if HELD_CAME[at].load(Ordering::Relaxed) {
            // SAFETY: kill has no memory-safety preconditions.
            unsafe { libc::kill(libc::getpid(), signal) };
        }
    }
}

/// The reload is over without an exec: whether a stop came while
/// [`arm_stop`] was in force.
pub fn disarm_stop() -> bool {
    let held = take_held_record();
    held.terminate || held.interrupt
}

/// Which of the held signals came while this process started.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct HeldSignals {
    pub reload: bool,
    pub hangup: bool,
    pub terminate: bool,
    pub interrupt: bool,
}

static HELD_CAME: [std::sync::atomic::AtomicBool; HELD.len()] =
    [const { std::sync::atomic::AtomicBool::new(false) }; HELD.len()];
static HELD_TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The handler chained behind the server's own ([`record_signals`]): only
/// notes which of [`HELD`] came, while the server starts and while a reload
/// gets ready for its exec ([`arm_stop`]). Async-signal-safe: atomics only.
extern "C" fn record_held(signal: libc::c_int) {
    use std::sync::atomic::Ordering;
    if HELD_TAKEN.load(Ordering::Relaxed) {
        return;
    }
    if let Some(at) = HELD.iter().position(|&held| held == signal) {
        HELD_CAME[at].store(true, Ordering::Relaxed);
    }
}

/// First thing in a process: if it was started with [`HELD`] blocked (a
/// supervisor exec'd by a reload), note which of them come from now on
/// with a handler that only records them, and unblock them. A signal that
/// arrived during the exec is recorded at once. A server then blocks them
/// again on all its threads but one ([`serve_signals`]); its children start
/// with nothing blocked ([`spawn_unblocked`]). The supervisor acts on the
/// record ([`take_held_record`]) once its own handlers are in place, which
/// call this one too.
pub fn take_held_signals() {
    // SAFETY: plain libc calls with valid arguments, while the process has
    // a single thread.
    unsafe {
        let mut blocked: libc::sigset_t = std::mem::zeroed();
        libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), &mut blocked);
        let held: Vec<libc::c_int> = HELD
            .into_iter()
            .filter(|&signal| libc::sigismember(&blocked, signal) == 1)
            .collect();
        if held.is_empty() {
            return;
        }
        install_recorder(&held);
        release_held_signals();
    }
}

/// For a server starting afresh, before it binds anything: record [`HELD`]
/// from now on whatever the mask, so a SIGTERM, SIGINT, SIGHUP or SIGUSR1
/// before the supervisor's handlers is acted on once they are in place
/// ([`take_held_record`]) instead of ending the server. A disposition is
/// process-wide: it covers the runtime's threads started later.
pub fn record_signals() {
    install_recorder(&HELD);
}

/// Install [`record_held`] for `signals`.
fn install_recorder(signals: &[libc::c_int]) {
    for &signal in signals {
        // SAFETY: a zeroed sigaction with a valid handler and an empty mask.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = record_held as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_flags = libc::SA_RESTART;
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// What [`take_held_signals`] recorded; from now on it records nothing.
pub fn take_held_record() -> HeldSignals {
    use std::sync::atomic::Ordering;
    HELD_TAKEN.store(true, Ordering::Relaxed);
    let came = |at: usize| HELD_CAME[at].swap(false, Ordering::Relaxed);
    HeldSignals {
        reload: came(0),
        hangup: came(1),
        terminate: came(2),
        interrupt: came(3),
    }
}

/// Where this supervisor puts the copy of its state its successor is
/// checked with.
pub fn check_path() -> PathBuf {
    clipboard::runtime_dir().join(format!("server-{}.check.state", std::process::id()))
}

/// Where the pane process of pane `pane_id` puts the copy of its state its
/// successor is checked with.
pub fn pane_check_path(pane_id: &Uuid) -> PathBuf {
    pane_state_dir().join(format!("{pane_id}.check.state"))
}

/// Read the copy of a state a check child is given. It is left where it is:
/// the process that asked for the check removes it.
pub fn read_check_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("read {}", path.display()))
}

/// Where this supervisor leaves its state for the one it execs. An exec
/// keeps the pid, so the successor finds the file by its own.
fn state_path() -> PathBuf {
    clipboard::runtime_dir().join(format!("server-{}.state", std::process::id()))
}

/// Where the pane process of pane `pane_id` leaves its state for the one it
/// execs, which is started with the same id.
pub fn pane_state_path(pane_id: &Uuid) -> PathBuf {
    pane_state_dir().join(format!("{pane_id}.state"))
}

fn pane_state_dir() -> PathBuf {
    clipboard::runtime_dir().join("panes")
}

/// Write a state file only the user can read. It holds what is on a screen
/// and what was typed but not yet delivered.
///
/// The file appears whole or not at all: the bytes go to a temporary file
/// next to it (`.<name>.tmp`), synced, then renamed over it. A write that
/// fails (a full disk) removes the temporary file, so nothing half-written
/// is left for a successor or for the sweep.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = format!(
        ".{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    write_via(path, &temp, bytes)
}

/// Write a file every server of the user shares (the shell wrappers) as
/// [`write_file`] does, unless it already holds `bytes`: a running server's
/// panes read it while another starts. The temporary file is this write's
/// own (`.<name>.<pid>-<n>.tmp`), so two servers writing at once do not
/// rename each other's.
pub fn write_shared_file(path: &Path, bytes: &[u8]) -> Result<()> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    let temp = format!(
        ".{}.{}-{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    write_via(path, &temp, bytes)
}

fn write_via(path: &Path, temp: &str, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    clipboard::private_dir(dir)?;
    let temp = dir.join(temp);
    let written = File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, path));
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(err).with_context(|| format!("write {}", path.display()));
    }
    Ok(())
}

/// Read a state file and remove it: it is for one successor, once. `None`
/// if there is none, which is how a process knows it starts afresh.
pub fn take_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    Ok(Some(bytes))
}

/// Leave the state for the supervisor this one is about to exec. Returns the
/// file, to be removed if the exec does not happen.
pub fn write_state(state: &SupervisorState) -> Result<PathBuf> {
    let path = state_path();
    write_file(&path, &state.encode_to_vec())?;
    Ok(path)
}

/// The state the previous supervisor left for this one. `None` if this
/// process was not exec'd by a supervisor.
pub fn take_state() -> Result<Option<SupervisorState>> {
    take_file(&state_path())?
        .map(|bytes| decode_state(&bytes))
        .transpose()
}

pub fn decode_state(bytes: &[u8]) -> Result<SupervisorState> {
    let state = SupervisorState::decode(bytes).context("decode state")?;
    if !format_matches(state.format, STATE_FORMAT) {
        bail!(
            "state is in format {}, this server reads {STATE_FORMAT}",
            state.format
        );
    }
    Ok(state)
}

/// Remove state files nobody came for: a process that died between writing
/// its state and its successor reading it leaves one behind. A reload takes
/// moments, so a file this old belongs to no reload under way.
pub fn sweep_stale() {
    let runtime_dir = clipboard::runtime_dir();
    for (dir, prefix) in [(runtime_dir, "server-"), (pane_state_dir(), "")] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // A state file, or the temporary file of one whose writer died.
            let state = name.starts_with(prefix) && name.ends_with(".state")
                || name.starts_with(&format!(".{prefix}")) && name.ends_with(".state.tmp");
            let stale = state
                && entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .is_ok_and(|at| at.elapsed().is_ok_and(|age| age > STALE_AFTER));
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_state_formats_obeys_the_selected_policy() {
        for setting in [None, Some("true"), Some("1")] {
            let unbounded = cfg!(feature = "failpoints") && setting == Some("1");
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "reload::tests::delayed_formats_fixture",
                    "--nocapture",
                ])
                .env(
                    "TILR_DELAYED_FORMATS_FIXTURE",
                    if unbounded { "unbounded" } else { "bounded" },
                )
                .env_remove("TILR_TEST_STATE_FORMATS_UNBOUNDED");
            if let Some(setting) = setting {
                command.env("TILR_TEST_STATE_FORMATS_UNBOUNDED", setting);
            }
            let output = command.output().unwrap();
            assert!(output.status.success(), "unbounded={unbounded}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
                "the controlled-delay fixture must actually run: {output:?}"
            );
        }
    }

    #[tokio::test]
    async fn delayed_formats_fixture() {
        use std::future::Future;
        use std::os::unix::fs::PermissionsExt;
        use std::task::Poll;
        let Ok(policy) = std::env::var("TILR_DELAYED_FORMATS_FIXTURE") else {
            return;
        };
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(scratch("delayed-formats"));
        let dir = &scratch.0;
        let ready = dir.join("ready");
        let release = dir.join("release");
        assert!(
            std::process::Command::new("mkfifo")
                .args([&ready, &release])
                .status()
                .unwrap()
                .success()
        );
        let mut release_writer = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&release)
            .unwrap();
        let exe = dir.join("successor");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf ready > '{}'\nread release < '{}'\nprintf '%s' '{}'\n",
                ready.display(),
                release.display(),
                formats()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut probe = Box::pin(check_executable(&exe));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(probe.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        // FIFO EOF proves that the real successor has started and is held
        // before reporting its formats. No wall-clock delay is required.
        assert_eq!(tokio::fs::read(&ready).await.unwrap(), b"ready");
        tokio::time::pause();
        tokio::time::advance(CHECK_TIMEOUT + Duration::from_secs(1)).await;
        let verdict = std::future::poll_fn(|cx| Poll::Ready(probe.as_mut().poll(cx))).await;
        if policy == "unbounded" {
            assert!(
                verdict.is_pending(),
                "the compatibility probe used the production deadline"
            );
            release_writer.write_all(b"continue\n").unwrap();
            assert!(probe.await.unwrap());
        } else {
            let Poll::Ready(Err(err)) = verdict else {
                panic!("a held successor must time out: {verdict:?}");
            };
            assert!(
                err.to_string().contains("state-formats did not finish"),
                "{err:#}"
            );
        }
    }

    #[test]
    fn an_executable_reading_the_same_formats_can_take_over() {
        assert!(
            check_formats(&formats()).unwrap(),
            "it takes over the Unix socket"
        );
    }

    #[test]
    fn check_formats_rejects_every_format_mismatch() {
        // What a successor prints for `state-formats`: this server's own
        // formats with one thing changed, or nothing usable at all.
        let with = |from: &str, to: &str| {
            let printed = formats().replace(from, to);
            assert_ne!(printed, formats(), "`{from}` is not in the formats");
            printed
        };
        let supervisor = format!("{SUPERVISOR_STATE} {STATE_FORMAT}");
        let (pane_written, pane_read) = (crate::pane::STATE_FORMAT, crate::pane::STATE_FORMAT + 1);
        let supervisor_read = STATE_FORMAT + 1;
        let cases = [
            (
                "another supervisor state",
                with(
                    &supervisor,
                    &format!("{SUPERVISOR_STATE} {supervisor_read}"),
                ),
                format!(
                    "reads supervisor-state format {supervisor_read}, this server writes {STATE_FORMAT}"
                ),
            ),
            (
                "another pane state",
                with(
                    &format!("{PANE_STATE} {pane_written}"),
                    &format!("{PANE_STATE} {pane_read}"),
                ),
                format!("reads pane-state format {pane_read}, this server writes {pane_written}"),
            ),
            (
                "a release before the reload check",
                with("reload-check 1\n", ""),
                "cannot show that it can resume (no `reload-check 1`)".to_string(),
            ),
            (
                "no formats at all",
                String::new(),
                "does not say which supervisor-state format it reads".to_string(),
            ),
            (
                "something else than formats",
                "tilrd 0.1.0\n".to_string(),
                "does not say which supervisor-state format it reads".to_string(),
            ),
            (
                "no pane state",
                with(&format!("{PANE_STATE} {pane_written}\n"), ""),
                "does not say which pane-state format it reads".to_string(),
            ),
            (
                "a format that is not a number",
                with(&supervisor, &format!("{SUPERVISOR_STATE} one")),
                "supervisor-state format: invalid digit".to_string(),
            ),
        ];
        for (case, printed, refusal) in cases {
            let err = match check_formats(&printed) {
                Ok(_) => panic!("{case}: taken over from {printed:?}"),
                Err(err) => format!("{err:#}"),
            };
            assert!(err.contains(&refusal), "{case}: {err}");
        }
    }

    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        use std::os::unix::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_check_passes_only_on_ok_and_a_clean_exit() {
        assert!(verdict(&output(0, "ok\n", "")).is_ok());
        let err = verdict(&output(1, "ok\n", "")).unwrap_err().to_string();
        assert!(err.contains("answered ok but ended with"), "{err}");
        let err = verdict(&output(1, "error: the configuration: bad key\n", ""))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "the configuration: bad key");
    }

    #[test]
    fn a_check_child_that_dies_is_reported_with_what_it_said_last() {
        let stderr = "zero\none\n\ntwo\nerror: unexpected argument '--socket' found\n";
        let err = verdict(&output(2, "", stderr)).unwrap_err().to_string();
        assert!(err.contains("without answering"), "{err}");
        assert!(err.contains("unexpected argument '--socket'"), "{err}");
        assert!(!err.contains("zero"), "only the last lines: {err}");
        let err = verdict(&output(2, "", "")).unwrap_err().to_string();
        assert!(err.ends_with("without answering"), "{err}");
    }

    fn state(format: u32) -> Vec<u8> {
        SupervisorState {
            listener: 7,
            format,
            ..Default::default()
        }
        .encode_to_vec()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tilr-reload-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_state_file_is_replaced_whole_and_nothing_else_is_left() {
        let dir = scratch("whole");
        let path = dir.join("server-1.state");
        write_file(&path, b"first").unwrap();
        write_file(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_eq!(names(&dir), ["server-1.state"]);
        assert_eq!(
            std::fs::metadata(&path).unwrap().mode() & 0o777,
            0o600,
            "only the user reads it"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_state_file_that_cannot_be_written_leaves_nothing_behind() {
        let dir = scratch("fails");
        // A directory where the file would go: the rename fails, as a
        // write to a full disk would.
        let path = dir.join("server-1.state");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), b"x").unwrap();
        assert!(write_file(&path, b"state").is_err());
        assert_eq!(names(&dir), ["server-1.state"], "no temporary file");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn state_in_the_current_format_is_read() {
        assert_eq!(decode_state(&state(STATE_FORMAT)).unwrap().listener, 7);
    }

    #[test]
    fn state_in_another_format_is_refused() {
        for format in [0, STATE_FORMAT - 1, STATE_FORMAT + 1] {
            let err = decode_state(&state(format)).unwrap_err();
            assert!(
                err.to_string().contains(&format!("format {format}")),
                "{err}"
            );
        }
    }
}
