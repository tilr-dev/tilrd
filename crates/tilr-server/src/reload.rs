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
//! Client connections do not survive: clients reconnect and find their panes
//! as they left them.

use std::fs::File;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use prost::Message;
use tilr_proto::SupervisorState;
use uuid::Uuid;

use crate::clipboard;

/// How often the executable on disk is checked for a change.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
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

/// What `tilr-server state-formats` prints: the versions of the state files
/// this executable reads, one `name version` per line.
pub fn formats() -> String {
    format!(
        "{SUPERVISOR_STATE} {STATE_FORMAT}\n{PANE_STATE} {}\n",
        crate::pane::STATE_FORMAT
    )
}

const SUPERVISOR_STATE: &str = "supervisor-state";
const PANE_STATE: &str = "pane-state";

/// Whether state written as version `written` can be read as version
/// `read`. Version 0 is state from before versions were recorded, which has
/// the layout of version 1.
pub fn format_matches(written: u32, read: u32) -> bool {
    written.max(1) == read
}

/// Check what another executable printed for `state-formats` against the
/// state this one writes.
fn check_formats(printed: &str) -> Result<()> {
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
    Ok(())
}

/// Make sure the executable on disk runs and reads the state this server
/// would leave it, before betting every pane on it.
pub async fn check_executable(exe: &Path) -> Result<()> {
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
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        clipboard::private_dir(dir)?;
    }
    File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .with_context(|| format!("write {}", path.display()))
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

fn decode_state(bytes: &[u8]) -> Result<SupervisorState> {
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
            let stale = name.starts_with(prefix)
                && name.ends_with(".state")
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
    fn an_executable_that_names_no_formats_cannot_take_over() {
        assert!(check_formats("").is_err());
        assert!(check_formats("tilr-server 0.1.0\n").is_err());
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
