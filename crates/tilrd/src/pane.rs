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

use std::collections::VecDeque;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use prost::Message;
use tilr_proto::{
    ClientMessage, FrameTiming, PaneExited, PaneHandover, PendingInput, ServerMessage,
    client_message::Msg, server_message,
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
pub const STATE_FORMAT: u32 = 3;

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
    /// Bytes written to the pty so far (by this process), to tell when a
    /// marked input has gone through.
    written: u64,
    /// Input messages named by a `seq`, oldest first, until their last byte
    /// is written to the pty.
    inputs: VecDeque<InputMark>,
    /// The `seq` of the input credited last: `Frame.last_input_seq`.
    last_input_seq: u64,
    /// An input was credited since the last frame: the next frame goes out
    /// even if the screen did not change, to carry its `seq`.
    input_credited: bool,
    /// What went into the next frame, for its `FrameTiming`.
    stats: FrameStats,
    /// How long encoding the last frame took; reported with the next one.
    last_encode: Duration,
}

/// An input message the client named by `ClientMessage.seq`.
struct InputMark {
    seq: u64,
    /// Through once `Pane::written` reaches this.
    end: u64,
    received: Instant,
}

/// Measurements of what led to the next frame. A few clock readings per
/// read and per frame, nothing more.
#[derive(Default)]
struct FrameStats {
    /// The first pty output since the last frame was built.
    first_output_at: Option<Instant>,
    /// Feeding output to the screen model, summed.
    parse: Duration,
    output_bytes: u64,
    reads: u32,
    /// `FrameTiming.input_us` of the input that last went through.
    input_us: u32,
}

/// A duration in whole microseconds, `u32::MAX` if it does not fit.
fn micros(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros()).unwrap_or(u32::MAX)
}

async fn serve(args: Args) -> Result<()> {
    let mut to_supervisor = tokio::io::stdout();
    let mut pending_input = Vec::new();
    let id = args.id.to_string();
    let mut last_input_seq = 0;
    let mut inputs = VecDeque::new();
    let (master, shell, screen, seq) = if let Some(handover) = take_handover(&args.id)? {
        crate::fail::point("pane-resume")?;
        crate::fail::point(&format!("pane-resume:{id}"))?;
        // SAFETY: the process that exec'd this one left the descriptor open
        // for it and nothing else here owns it.
        let master = unsafe { OwnedFd::from_raw_fd(handover.master) };
        pty::set_cloexec(master.as_raw_fd(), true)?;
        pending_input = handover.pending_input;
        last_input_seq = handover.last_input_seq;
        let now = Instant::now();
        inputs = handover
            .pending_inputs
            .iter()
            .map(|mark| InputMark {
                seq: mark.seq,
                end: mark.end,
                received: now
                    .checked_sub(Duration::from_micros(mark.age_us))
                    .unwrap_or(now),
            })
            .collect();
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
            pty::Process::inherited(handover.shell_pid),
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
        // SAFETY: `master` is an `OwnedFd`, so its descriptor stays open and unchanged
        // until the `AsyncFd` drops it, and the pane never swaps it out.
        pty: unsafe { AsyncFd::register(master) }.context("register pty")?,
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
        // The pending input handed over counts from 0, as the marks do.
        written: 0,
        inputs,
        last_input_seq,
        input_credited: false,
        stats: FrameStats::default(),
        last_encode: Duration::ZERO,
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
                    Ok(Ok(n)) => pane.on_written(n),
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
        let start = Instant::now();
        self.screen.feed(bytes);
        self.pending_input.extend(self.screen.take_replies());
        let stats = &mut self.stats;
        stats.parse += start.elapsed();
        stats.first_output_at.get_or_insert(start);
        stats.output_bytes += bytes.len() as u64;
        stats.reads = stats.reads.saturating_add(1);
        self.dirty = true;
    }

    /// The pty took the first `n` bytes of the pending input.
    fn on_written(&mut self, n: usize) {
        self.pending_input.drain(..n);
        self.written += n as u64;
        self.settle_inputs();
    }

    /// Note an input message named by `seq`: it is credited once what is
    /// pending now has been written to the pty.
    fn mark_input(&mut self, seq: u64, received: Instant) {
        self.inputs.push_back(InputMark {
            seq,
            end: self.written + self.pending_input.len() as u64,
            received,
        });
        self.settle_inputs();
    }

    /// Credit the marked inputs whose bytes are all written, in the order
    /// they came.
    fn settle_inputs(&mut self) {
        while let Some(mark) = self.inputs.front()
            && mark.end <= self.written
        {
            self.credit_input(mark.seq, mark.received);
            self.inputs.pop_front();
        }
    }

    /// The input named by `seq` has taken effect: the next frame carries it,
    /// whether or not the screen changes.
    fn credit_input(&mut self, seq: u64, received: Instant) {
        self.last_input_seq = seq;
        self.stats.input_us = micros(received.elapsed());
        self.input_credited = true;
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
            last_input_seq: self.last_input_seq,
            // The successor counts written bytes from 0, from the start of
            // `pending_input`.
            pending_inputs: self
                .inputs
                .iter()
                .map(|mark| PendingInput {
                    seq: mark.seq,
                    end: mark.end.saturating_sub(self.written),
                    age_us: u64::try_from(mark.received.elapsed().as_micros()).unwrap_or(u64::MAX),
                })
                .collect(),
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
        /// How an input message is credited.
        enum Credit {
            /// Once its bytes are written to the pty.
            Written,
            /// When applied, which is at once.
            Applied,
        }
        let seq = msg.seq;
        let received = (seq != 0).then(Instant::now);
        let credit = match msg.msg {
            Some(Msg::Key(_) | Msg::Mouse(_) | Msg::Paste(_) | Msg::Raw(_)) => {
                Some(Credit::Written)
            }
            Some(Msg::Scroll(_) | Msg::Resize(_)) => Some(Credit::Applied),
            _ => None,
        };
        self.handle_message(msg)?;
        match (received, credit) {
            (Some(received), Some(Credit::Written)) => self.mark_input(seq, received),
            (Some(received), Some(Credit::Applied)) => self.credit_input(seq, received),
            _ => {}
        }
        Ok(())
    }

    fn handle_message(&mut self, msg: ClientMessage) -> Result<()> {
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
        let build_start = Instant::now();
        let built = match self.screen.frame(full)? {
            Some(frame) => Some(frame),
            // Input that changed nothing on the screen is answered all the
            // same, so the client knows it went through.
            None if self.input_credited => self.screen.unchanged_frame(),
            None => None,
        };
        let Some(mut frame) = built else {
            // The output seen so far waits for the frame that shows it, and
            // `wait_us` counts from the first of it.
            return Ok(());
        };
        let build = build_start.elapsed();
        self.input_credited = false;
        let stats = std::mem::take(&mut self.stats);
        let first_output_at = stats.first_output_at;
        self.seq += 1;
        frame.pane_id.clone_from(&self.id);
        frame.seq = self.seq;
        frame.last_input_seq = self.last_input_seq;
        frame.timing = Some(FrameTiming {
            wait_us: first_output_at
                .map_or(0, |at| micros(build_start.saturating_duration_since(at))),
            parse_us: micros(stats.parse),
            build_us: micros(build),
            encode_us: micros(self.last_encode),
            output_bytes: u32::try_from(stats.output_bytes).unwrap_or(u32::MAX),
            reads: stats.reads,
            input_us: stats.input_us,
        });
        let head = link::FrameHead {
            full: frame.full,
            last_input_seq: frame.last_input_seq,
        };
        let encode_start = Instant::now();
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::Frame(frame)));
        self.last_encode = encode_start.elapsed();
        link::write_tagged(
            &mut self.to_supervisor,
            link::kind::FRAME,
            &head.prepend(&packet),
        )
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
