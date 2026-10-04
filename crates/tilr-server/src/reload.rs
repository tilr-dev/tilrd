//! Picking up a new executable without closing any shell.
//!
//! The supervisor replaces itself with the executable now on disk, the same
//! way a pane process does: it execs in place, so its pid stays and the pane
//! processes stay its children. The listening socket and the pipes to the
//! pane processes carry over as open descriptors; what the supervisor knows
//! about the panes travels in an unlinked file. The new supervisor then asks
//! every pane process to reload too.
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

/// Make sure the executable on disk runs at all before betting every pane on
/// it.
pub async fn check_executable(exe: &Path) -> Result<()> {
    let status = tokio::process::Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status();
    match tokio::time::timeout(CHECK_TIMEOUT, status).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => bail!("{} --version: {status}", exe.display()),
        Ok(Err(err)) => Err(err).with_context(|| format!("run {}", exe.display())),
        Err(_) => bail!("{} --version did not finish", exe.display()),
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
    SupervisorState::decode(bytes.as_slice()).context("decode state")
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
