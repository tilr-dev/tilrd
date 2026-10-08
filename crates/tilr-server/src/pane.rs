//! Per-pane process: opens the pty, runs the shell on it, keeps the screen
//! model, turns it into frames, and turns client input into pty bytes.
//!
//! The pty and the shell live and die with this process. It talks to the
//! supervisor over its own stdin and stdout.
//!
//! New code is picked up without losing the shell: asked to reload, the
//! process execs the executable now on disk. The pty, the shell (still its
//! child) and the pipes to the supervisor carry over as they are; the screen
//! and the rest travel in a file named after the pane, which the new process
//! finds by the id it is started with.

use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use prost::Message;
use tilr_proto::{
    ClientMessage, PaneExited, PaneHandover, ServerMessage, client_message::Msg, server_message,
};
use tilr_screen::{GhosttyScreen, Screen};
use tokio::io::Stdout;
use tokio::io::unix::AsyncFd;
use tokio::net::unix::pipe;
use tokio::sync::mpsc;
use tokio::time::Instant;
use uuid::Uuid;

use crate::{link, pty, reload};

/// Minimum spacing between frames. Output arriving faster is coalesced.
const FRAME_INTERVAL: Duration = Duration::from_millis(8);
const READ_BUF: usize = 64 * 1024;
/// How long a shell gets to exit after SIGHUP before it is killed.
const KILL_GRACE: Duration = Duration::from_secs(2);

/// Size of a pane started without one.
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// How the pane process is started. A reload execs it with the same
/// arguments, so they describe how the pane began: a pane taking over from
/// another has its size from the state left for it.
#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    pub id: Uuid,
    #[arg(long, default_value_t = DEFAULT_COLS)]
    pub cols: u16,
    #[arg(long, default_value_t = DEFAULT_ROWS)]
    pub rows: u16,
    /// Working directory of the shell. Empty = this process's own.
    #[arg(long, default_value = "")]
    pub cwd: String,
    /// Command to run instead of the shell.
    #[arg(last = true)]
    pub command: Vec<String>,
}

/// Version of what a reloading pane process leaves its successor
/// ([`PaneHandover`]). Raise it with any change an older or newer pane
/// process would misread.
pub const STATE_FORMAT: u32 = 2;

/// The state the pane process that exec'd this one left for it. `None` if
/// this process starts a pane of its own.
fn take_handover(pane_id: &Uuid) -> Result<Option<PaneHandover>> {
    reload::take_file(&reload::pane_state_path(pane_id))?
        .map(|bytes| decode_handover(&bytes))
        .transpose()
}

fn decode_handover(bytes: &[u8]) -> Result<PaneHandover> {
    let handover = PaneHandover::decode(bytes).context("decode state")?;
    if !reload::format_matches(handover.format, STATE_FORMAT) {
        bail!(
            "state is in format {}, this pane process reads {STATE_FORMAT}",
            handover.format
        );
    }
    Ok(handover)
}

enum FromSupervisor {
    Packet(Bytes),
    Reload,
    /// The supervisor closed the pipe.
    Closed,
}

/// Read packets from the supervisor into `tx`. Reading a length-prefixed
/// packet is not cancel-safe, so it gets its own task and the main loop
/// selects on a channel instead.
///
/// The task ends with a reload request, leaving the rest of the pipe unread
/// for the process that takes over.
fn read_supervisor(tx: mpsc::Sender<FromSupervisor>) {
    tokio::spawn(async move {
        let mut from_supervisor = match supervisor_pipe() {
            Ok(pipe) => pipe,
            Err(err) => {
                tracing::error!("cannot read from the supervisor: {err}");
                let _ = tx.send(FromSupervisor::Closed).await;
                return;
            }
        };
        loop {
            let (message, last) = match link::read_packet(&mut from_supervisor).await {
                Ok(Some(packet)) if packet.is_empty() => (FromSupervisor::Reload, true),
                Ok(Some(packet)) => (FromSupervisor::Packet(packet), false),
                _ => (FromSupervisor::Closed, true),
            };
            if tx.send(message).await.is_err() || last {
                break;
            }
        }
    });
}

/// The pipe from the supervisor (descriptor 0), read without a buffer.
///
/// The standard input handle would read ahead, and what it had read past a
/// reload request would be lost to the process that takes over. The reader
/// works on a duplicate, so that dropping it leaves descriptor 0 open for
/// that process.
fn supervisor_pipe() -> std::io::Result<pipe::Receiver> {
    // SAFETY: dup has no memory-safety preconditions.
    let fd = unsafe { libc::dup(0) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: dup returned a fresh descriptor that we now own.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    pty::set_cloexec(fd.as_raw_fd(), true)?;
    pipe::Receiver::from_owned_fd(fd)
}

pub fn run(args: Args) -> Result<()> {
    // The screen model is not Send, so everything runs on this one thread.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(serve(args));
    // The stdin reader may sit in a blocking read that only ends when the
    // supervisor closes the pipe; exit without waiting for it.
    runtime.shutdown_background();
    result
}

struct Pane {
    /// As it goes into every frame.
    id: String,
    /// Where a reload leaves the state for the process that takes over.
    state_path: PathBuf,
    pty: AsyncFd<OwnedFd>,
    screen: GhosttyScreen,
    to_supervisor: Stdout,
    shell: pty::Process,
    /// When to kill a shell that was asked to close and has not exited.
    kill_at: Option<Instant>,
    /// Input waiting for the pty to accept it.
    pending_input: Vec<u8>,
    seq: u64,
    dirty: bool,
    want_full: bool,
    last_frame: Instant,
}

async fn serve(args: Args) -> Result<()> {
    let mut to_supervisor = tokio::io::stdout();
    let mut pending_input = Vec::new();
    let id = args.id.to_string();
    let (master, shell, screen, seq) = if let Some(handover) = take_handover(&args.id)? {
        crate::fail::point("pane-resume")?;
        crate::fail::point(&format!("pane-resume:{id}"))?;
        // SAFETY: the process that exec'd this one left the descriptor open
        // for it and nothing else here owns it.
        let master = unsafe { OwnedFd::from_raw_fd(handover.master) };
        pty::set_cloexec(master.as_raw_fd(), true)?;
        pending_input = handover.pending_input;
        let size = |cells: u32| cells.clamp(1, u32::from(u16::MAX)) as u16;
        let cell = if handover.cell_width_px > 0 && handover.cell_height_px > 0 {
            (handover.cell_width_px, handover.cell_height_px)
        } else {
            tilr_screen::DEFAULT_CELL_PX
        };
        let mut screen = GhosttyScreen::restore(
            size(handover.cols),
            size(handover.rows),
            cell,
            &handover.snapshot,
        )?;
        // Whoever was reading history keeps their place.
        if handover.scrolled_back > 0 {
            screen.scroll(-(handover.scrolled_back.min(i32::MAX as u32) as i32));
        }
        tracing::info!(pane = id, "resumed after reload");
        (
            master,
            pty::Process::Inherited(handover.shell_pid),
            screen,
            handover.seq,
        )
    } else {
        let pty::Spawned { master, child } =
            match pty::spawn_shell(&id, &args.command, &args.cwd, args.cols, args.rows) {
                Ok(spawned) => spawned,
                Err(err) => {
                    let packet = tilr_proto::encode(&tilr_proto::Error {
                        message: format!("{err:#}"),
                    });
                    return link::write_tagged(&mut to_supervisor, link::kind::FAILED, &packet)
                        .await
                        .context("report failure to supervisor");
                }
            };
        pty::set_nonblocking(master.as_raw_fd())?;
        link::write_tagged(&mut to_supervisor, link::kind::STARTED, &[])
            .await
            .context("report start to supervisor")?;
        let screen = GhosttyScreen::new(args.cols, args.rows)?;
        (master, pty::Process::Spawned(child), screen, 0)
    };

    let (packet_tx, mut packets) = mpsc::channel(256);
    read_supervisor(packet_tx.clone());

    let mut pane = Pane {
        id,
        state_path: reload::pane_state_path(&args.id),
        pty: AsyncFd::new(master).context("register pty")?,
        screen,
        to_supervisor,
        shell,
        kill_at: None,
        pending_input,
        seq,
        // A resumed pane redraws its clients from the restored screen.
        dirty: seq > 0,
        want_full: seq > 0,
        last_frame: Instant::now() - FRAME_INTERVAL,
    };
    let mut buf = vec![0u8; READ_BUF];

    loop {
        tokio::select! {
            guard = pane.pty.readable() => {
                let read = guard?.try_io(|fd| pty::read(fd.as_raw_fd(), &mut buf));
                match read {
                    // EOF, or EIO once the last slave descriptor is closed:
                    // the shell is gone.
                    Ok(Ok(0)) => break,
                    Ok(Err(err)) if err.raw_os_error() == Some(libc::EIO) => break,
                    Ok(Ok(n)) => pane.on_output(&buf[..n]),
                    Ok(Err(err)) => return Err(err).context("read pty"),
                    Err(_would_block) => {}
                }
            }
            guard = pane.pty.writable(), if !pane.pending_input.is_empty() => {
                let input = &pane.pending_input;
                let written = guard?.try_io(|fd| pty::write(fd.as_raw_fd(), input));
                match written {
                    Ok(Ok(n)) => { pane.pending_input.drain(..n); }
                    Ok(Err(err)) if err.raw_os_error() == Some(libc::EIO) => break,
                    Ok(Err(err)) => return Err(err).context("write pty"),
                    Err(_would_block) => {}
                }
            }
            packet = packets.recv() => match packet {
                Some(FromSupervisor::Packet(packet)) => {
                    match tilr_proto::decode::<ClientMessage>(&packet) {
                        Ok(msg) => pane.on_message(msg)?,
                        Err(err) => tracing::warn!(pane = pane.id, "bad packet from supervisor: {err}"),
                    }
                }
                Some(FromSupervisor::Reload) => {
                    // Only returns if this process carries on.
                    let err = pane.reload();
                    tracing::error!(pane = pane.id, "reload failed: {err:#}");
                    read_supervisor(packet_tx.clone());
                }
                // The supervisor is gone: nothing left to serve. Exiting
                // closes the pty, which hangs up the shell.
                Some(FromSupervisor::Closed) | None => return Ok(()),
            },
            () = tokio::time::sleep_until(pane.last_frame + FRAME_INTERVAL), if pane.dirty => {
                pane.send_frame().await?;
            }
            () = tokio::time::sleep_until(pane.kill_at.unwrap_or_else(Instant::now)), if pane.kill_at.is_some() => {
                pane.kill_at = None;
                pane.signal_shell(libc::SIGKILL);
            }
        }
    }

    // Show whatever the shell printed last.
    pane.send_frame().await?;
    let exit_code = match tokio::time::timeout(KILL_GRACE, pane.shell.wait()).await {
        Ok(code) => code.unwrap_or(-1),
        // The pty is closed but the shell lingers.
        Err(_) => {
            pane.signal_shell(libc::SIGKILL);
            -1
        }
    };
    let packet = tilr_proto::encode(&PaneExited {
        pane_id: pane.id.clone(),
        exit_code,
    });
    link::write_tagged(&mut pane.to_supervisor, link::kind::EXITED, &packet)
        .await
        .context("report exit to supervisor")
}

impl Pane {
    fn on_output(&mut self, bytes: &[u8]) {
        self.screen.feed(bytes);
        self.pending_input.extend(self.screen.take_replies());
        self.dirty = true;
    }

    fn signal_shell(&self, signal: libc::c_int) {
        if let Some(pid) = self.shell.pid() {
            pty::signal_group(pid, signal);
        }
    }

    /// Replace this process with the executable now on disk, which resumes
    /// the pane where this one stops. Returns only on failure, with the pane
    /// still in working order.
    fn reload(&mut self) -> anyhow::Error {
        let (cols, rows) = self.screen.size();
        let cell = self.screen.cell_px();
        let scrolled_back = self.screen.scrolled_back();
        let snapshot = match GhosttyScreen::new(cols, rows) {
            Ok(blank) => std::mem::replace(&mut self.screen, blank).snapshot(),
            Err(err) => return err,
        };
        self.want_full = true;
        self.dirty = true;
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(err) => return err.context("snapshot screen (its contents are lost)"),
        };
        let err = self.exec_successor(cols, rows, cell, &snapshot, scrolled_back);
        match GhosttyScreen::restore(cols, rows, cell, &snapshot) {
            Ok(mut screen) => {
                screen.scroll(-(scrolled_back.min(i32::MAX as u32) as i32));
                self.screen = screen;
            }
            Err(restore) => tracing::error!(pane = self.id, "screen lost: {restore:#}"),
        }
        err
    }

    fn exec_successor(
        &self,
        cols: u16,
        rows: u16,
        cell: (u32, u32),
        snapshot: &[u8],
        scrolled_back: u32,
    ) -> anyhow::Error {
        let Some(shell_pid) = self.shell.pid() else {
            return anyhow::anyhow!("the shell has already exited");
        };
        if let Err(err) = crate::fail::point("pane-reload-before-exec") {
            return err;
        }
        let exe = crate::exe();
        let master = self.pty.get_ref().as_raw_fd();
        let handover = PaneHandover {
            format: STATE_FORMAT,
            master,
            shell_pid,
            seq: self.seq,
            scrolled_back,
            cols: u32::from(cols),
            rows: u32::from(rows),
            pending_input: self.pending_input.clone(),
            snapshot: snapshot.to_vec(),
            cell_width_px: cell.0,
            cell_height_px: cell.1,
        };
        let state = &self.state_path;
        if let Err(err) = reload::write_file(state, &handover.encode_to_vec()) {
            return err.context("write resume state");
        }
        // The pty must survive the exec.
        let err = match pty::set_cloexec(master, false) {
            Ok(()) => {
                tracing::info!(pane = self.id, exe = %exe.display(), "reloading");
                let err = std::process::Command::new(exe)
                    .args(std::env::args_os().skip(1))
                    .exec();
                anyhow::Error::new(err).context(format!("exec {}", exe.display()))
            }
            Err(err) => anyhow::Error::new(err).context("make the pty inheritable"),
        };
        let _ = pty::set_cloexec(master, true);
        let _ = std::fs::remove_file(state);
        err
    }

    fn on_message(&mut self, msg: ClientMessage) -> Result<()> {
        match msg.msg {
            Some(Msg::ClosePane(_)) => {
                self.signal_shell(libc::SIGHUP);
                self.kill_at.get_or_insert(Instant::now() + KILL_GRACE);
            }
            Some(Msg::Subscribe(_)) => {
                self.want_full = true;
                self.dirty = true;
            }
            Some(Msg::Resize(resize)) => {
                let cols = resize.cols.clamp(1, u32::from(u16::MAX)) as u16;
                let rows = resize.rows.clamp(1, u32::from(u16::MAX)) as u16;
                let cell = if resize.cell_width_px > 0 && resize.cell_height_px > 0 {
                    (resize.cell_width_px, resize.cell_height_px)
                } else {
                    self.screen.cell_px()
                };
                if (cols, rows) != self.screen.size() || cell != self.screen.cell_px() {
                    pty::set_window_size(self.pty.get_ref().as_fd(), cols, rows)
                        .context("set pty size")?;
                    self.screen.resize(cols, rows, cell)?;
                    self.want_full = true;
                    self.dirty = true;
                }
            }
            // A single input event that cannot be encoded is dropped; it must
            // never take the pane down with it.
            Some(Msg::Key(key)) => {
                self.screen.scroll_to_bottom();
                if let Err(err) = self.screen.encode_key(&key, &mut self.pending_input) {
                    tracing::warn!(pane = self.id, "dropped key event: {err:#}");
                }
                self.dirty = true;
            }
            Some(Msg::Mouse(mouse)) => {
                if let Err(err) = self.screen.encode_mouse(&mouse, &mut self.pending_input) {
                    tracing::warn!(pane = self.id, "dropped mouse event: {err:#}");
                }
            }
            Some(Msg::Paste(paste)) => {
                self.screen.scroll_to_bottom();
                if let Err(err) = self
                    .screen
                    .encode_paste(&paste.text, &mut self.pending_input)
                {
                    tracing::warn!(pane = self.id, "dropped paste: {err:#}");
                }
                self.dirty = true;
            }
            Some(Msg::Raw(raw)) => self.pending_input.extend_from_slice(&raw.data),
            Some(Msg::Scroll(scroll)) => {
                if scroll.to_bottom {
                    self.screen.scroll_to_bottom();
                } else {
                    self.screen.scroll(scroll.delta);
                }
                self.dirty = true;
            }
            // Everything else is handled by the supervisor.
            _ => {}
        }
        Ok(())
    }

    async fn send_frame(&mut self) -> Result<()> {
        self.dirty = false;
        self.last_frame = Instant::now();
        if let Some(status) = self.screen.take_status() {
            let packet = tilr_proto::encode(&status);
            link::write_tagged(&mut self.to_supervisor, link::kind::STATUS, &packet)
                .await
                .context("send status to supervisor")?;
        }
        let full = std::mem::take(&mut self.want_full);
        let Some(mut frame) = self.screen.frame(full)? else {
            return Ok(());
        };
        self.seq += 1;
        frame.pane_id.clone_from(&self.id);
        frame.seq = self.seq;
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::Frame(frame)));
        link::write_tagged(&mut self.to_supervisor, link::kind::RELAY, &packet)
            .await
            .context("send frame to supervisor")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover(format: u32) -> Vec<u8> {
        PaneHandover {
            format,
            shell_pid: 500,
            ..Default::default()
        }
        .encode_to_vec()
    }

    #[test]
    fn state_in_the_current_format_is_read() {
        let handover = decode_handover(&handover(STATE_FORMAT)).unwrap();
        assert_eq!(handover.shell_pid, 500);
    }

    #[test]
    fn state_in_another_format_is_refused() {
        for format in [0, STATE_FORMAT - 1, STATE_FORMAT + 1] {
            let err = decode_handover(&handover(format)).unwrap_err();
            assert!(
                err.to_string().contains(&format!("format {format}")),
                "{err}"
            );
        }
    }
}
