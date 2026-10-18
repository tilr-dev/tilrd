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
    let output = tokio::process::Command::new(exe)
        .arg("state-formats")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(CHECK_TIMEOUT, output).await {
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
    let child = tokio::process::Command::new(exe)
        .args(args)
        .env(CHECK_ENV, &check_file.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("run {}", exe.display()))?;
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
/// If the exec fails, [`release_held_signals`] undoes it.
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

/// Unblock [`HELD`] on this thread again: the exec that was to carry them
/// over did not happen.
pub fn release_held_signals() {
    let set = held_set();
    // SAFETY: the set is valid.
    unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) };
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
/// arrived during the exec is recorded at once; the threads and children
/// started after this have nothing blocked. The supervisor acts on the
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
    let dir = path.parent().unwrap_or(Path::new("."));
    clipboard::private_dir(dir)?;
    let temp = dir.join(format!(
        ".{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
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
    fn an_executable_reading_the_same_formats_can_take_over() {
        assert!(check_formats(&formats()).is_ok());
    }

    #[test]
    fn an_executable_reading_another_format_cannot_take_over() {
        let pane = crate::pane::STATE_FORMAT;
        let newer = format!("supervisor-state {}\npane-state {pane}\n", STATE_FORMAT + 1);
        let err = check_formats(&newer).unwrap_err().to_string();
        assert!(err.contains("reads supervisor-state format"), "{err}");

        let newer = format!("supervisor-state {STATE_FORMAT}\npane-state {}\n", pane + 1);
        let err = check_formats(&newer).unwrap_err().to_string();
        assert!(err.contains("reads pane-state format"), "{err}");
    }

    #[test]
    fn an_executable_that_cannot_run_the_check_cannot_take_over() {
        let without: String = formats()
            .lines()
            .filter(|line| !line.starts_with(RELOAD_CHECK))
            .map(|line| format!("{line}\n"))
            .collect();
        let err = check_formats(&without).unwrap_err().to_string();
        assert!(err.contains("no `reload-check 1`"), "{err}");
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

    #[test]
    fn an_executable_that_names_no_formats_cannot_take_over() {
        assert!(check_formats("").is_err());
        assert!(check_formats("tilrd 0.1.0\n").is_err());
        assert!(check_formats("supervisor-state 1\n").is_err());
        assert!(check_formats("supervisor-state one\npane-state 1\n").is_err());
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
