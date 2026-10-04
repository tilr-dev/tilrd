//! Picking up a new executable without closing any shell.
//!
//! The supervisor replaces itself with the executable now on disk, the same
//! way a pane process does: it execs in place, so its pid stays and the pane
//! processes stay its children. The listening socket and the pipes to the
//! pane processes carry over as open descriptors; what the supervisor knows
//! about the panes travels in an unlinked file. The new supervisor then asks
//! every pane process to reload too.
//!
//! The state files have a format version. A supervisor asks the executable on
//! disk which versions it reads before handing over to it: once it has
//! exec'd there is no way back, and a successor that misreads the state
//! would lose every shell.
//!
//! Client connections do not survive: clients reconnect and find their panes
//! as they left them.

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use prost::Message;
use tilr_proto::SupervisorState;

/// How often the executable on disk is checked for a change.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

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
pub const STATE_FORMAT: u32 = 1;

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

/// Put the state into a file only reachable through the returned handle.
pub fn write_state(state: &SupervisorState) -> std::io::Result<File> {
    let path = std::env::temp_dir().join(format!("tilr-supervisor-{}.state", std::process::id()));
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    file.write_all(&state.encode_to_vec())?;
    file.rewind()?;
    Ok(file)
}

/// Read back what [`write_state`] wrote, from the descriptor the previous
/// supervisor left open.
pub fn read_state(fd: RawFd) -> Result<SupervisorState> {
    // SAFETY: the supervisor that exec'd this process left the descriptor
    // open for it and nothing else here owns it.
    let mut file = unsafe { File::from_raw_fd(fd) };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).context("read state file")?;
    decode_state(&bytes)
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

/// The arguments this process was started with, minus a `--resume` of its
/// own: what the successor gets, before its `--resume`.
pub fn own_args() -> Vec<OsString> {
    let mut kept = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--resume" {
            args.next();
        } else if !arg.to_str().is_some_and(|arg| arg.starts_with("--resume=")) {
            kept.push(arg);
        }
    }
    kept
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
        assert!(err.contains("supervisor-state format 2"), "{err}");

        let newer = format!("supervisor-state {STATE_FORMAT}\npane-state {}\n", pane + 1);
        let err = check_formats(&newer).unwrap_err().to_string();
        assert!(err.contains("pane-state format 2"), "{err}");
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
            next_pane_id: 7,
            format,
            ..Default::default()
        }
        .encode_to_vec()
    }

    #[test]
    fn state_in_the_current_format_is_read() {
        assert_eq!(decode_state(&state(STATE_FORMAT)).unwrap().next_pane_id, 7);
    }

    #[test]
    fn state_written_before_formats_were_recorded_is_read() {
        assert_eq!(decode_state(&state(0)).unwrap().next_pane_id, 7);
    }

    #[test]
    fn state_in_another_format_is_refused() {
        let err = decode_state(&state(STATE_FORMAT + 1)).unwrap_err();
        assert!(err.to_string().contains("format 2"), "{err}");
    }
}
