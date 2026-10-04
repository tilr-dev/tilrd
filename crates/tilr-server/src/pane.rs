//! Per-pane process: opens the pty, runs the shell on it, keeps the screen
//! model, turns it into frames, and turns client input into pty bytes.
//!
//! The pty and the shell live and die with this process. It talks to the
//! supervisor over its own stdin and stdout.
//!
//! New code is picked up without losing the shell: asked to reload, the
//! process execs the executable now on disk. The pty, the shell (still its
//! child) and the pipes to the supervisor carry over as they are; the screen
//! travels as a snapshot in an unlinked file.

use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use tilr_proto::{ClientMessage, PaneExited, ServerMessage, client_message::Msg, server_message};
use tilr_screen::{GhosttyScreen, Screen};
use tokio::io::Stdout;
use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::{link, pty};

/// Minimum spacing between frames. Output arriving faster is coalesced.
const FRAME_INTERVAL: Duration = Duration::from_millis(8);
const READ_BUF: usize = 64 * 1024;
/// How long a shell gets to exit after SIGHUP before it is killed.
const KILL_GRACE: Duration = Duration::from_secs(2);

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    pub id: u64,
    #[arg(long)]
    pub cols: u16,
    #[arg(long)]
    pub rows: u16,
    /// Working directory of the shell. Empty = this process's own.
    #[arg(long, default_value = "")]
    pub cwd: String,
    /// Take over the pty and shell of the process that exec'd this one.
    #[arg(long)]
    pub resume: Option<Resume>,
    /// Command to run instead of the shell.
    #[arg(last = true)]
    pub command: Vec<String>,
}

/// What a reloading pane process hands to its successor, as
/// `master:state:shell_pid:seq` on the command line.
#[derive(Clone)]
pub struct Resume {
    /// Descriptor of the pty master.
    master: RawFd,
    /// Descriptor of the state file: u32 LE length and bytes of the input
    /// not yet written to the pty, then the screen snapshot.
    state: RawFd,
    shell_pid: u32,
    seq: u64,
}

impl FromStr for Resume {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let mut fields = s.split(':');
        let mut next = || fields.next().context("expected master:state:shell_pid:seq");
        Ok(Self {
            master: next()?.parse()?,
            state: next()?.parse()?,
            shell_pid: next()?.parse()?,
            seq: next()?.parse()?,
        })
    }
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
        let mut from_supervisor = tokio::io::stdin();
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
    id: u64,
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
    let (master, shell, screen, seq) = if let Some(resume) = args.resume {
        // SAFETY: the process that exec'd this one left both descriptors
        // open for it and nothing else here owns them.
        let (master, state) = unsafe {
            (
                OwnedFd::from_raw_fd(resume.master),
                File::from_raw_fd(resume.state),
            )
        };
        pty::set_cloexec(master.as_raw_fd(), true)?;
        let snapshot = read_state(state, &mut pending_input).context("read resume state")?;
        let screen = GhosttyScreen::restore(args.cols, args.rows, &snapshot)?;
        tracing::info!(pane = args.id, "resumed after reload");
        (master, pty::Process::Inherited(resume.shell_pid), screen, resume.seq)
    } else {
        let pty::Spawned { master, child } =
            match pty::spawn_shell(&args.command, &args.cwd, args.cols, args.rows) {
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
        id: args.id,
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
        pane_id: pane.id,
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
        let err = self.exec_successor(cols, rows, &snapshot);
        match GhosttyScreen::restore(cols, rows, &snapshot) {
            Ok(screen) => self.screen = screen,
            Err(restore) => tracing::error!(pane = self.id, "screen lost: {restore:#}"),
        }
        err
    }

    fn exec_successor(&self, cols: u16, rows: u16, snapshot: &[u8]) -> anyhow::Error {
        let Some(shell_pid) = self.shell.pid() else {
            return anyhow::anyhow!("the shell has already exited");
        };
        let exe = crate::exe();
        let state = match write_state(self.id, &self.pending_input, snapshot) {
            Ok(state) => state,
            Err(err) => return anyhow::Error::new(err).context("write resume state"),
        };
        let master = self.pty.get_ref().as_raw_fd();
        // Both descriptors must survive the exec; the state file's is simply
        // dropped if it fails.
        let inheritable = pty::set_cloexec(master, false)
            .and_then(|()| pty::set_cloexec(state.as_raw_fd(), false));
        if let Err(err) = inheritable {
            return anyhow::Error::new(err).context("make descriptors inheritable");
        }
        let resume = format!("{master}:{}:{shell_pid}:{}", state.as_raw_fd(), self.seq);
        tracing::info!(pane = self.id, exe = %exe.display(), "reloading");
        let err = std::process::Command::new(exe)
            .arg("pane")
            .args(["--id", &self.id.to_string()])
            .args(["--cols", &cols.to_string()])
            .args(["--rows", &rows.to_string()])
            .args(["--resume", &resume])
            .exec();
        let _ = pty::set_cloexec(master, true);
        anyhow::Error::new(err).context(format!("exec {}", exe.display()))
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
                if (cols, rows) != self.screen.size() {
                    pty::set_window_size(self.pty.get_ref().as_fd(), cols, rows)
                        .context("set pty size")?;
                    self.screen.resize(cols, rows)?;
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
        frame.pane_id = self.id;
        frame.seq = self.seq;
        let packet = tilr_proto::encode(&ServerMessage::from(server_message::Msg::Frame(frame)));
        link::write_tagged(&mut self.to_supervisor, link::kind::RELAY, &packet)
            .await
            .context("send frame to supervisor")
    }
}

/// Put what the successor needs into a file only reachable through the
/// returned handle.
fn write_state(pane_id: u64, pending_input: &[u8], snapshot: &[u8]) -> std::io::Result<File> {
    let path = std::env::temp_dir().join(format!("tilr-pane-{pane_id}-{}.state", std::process::id()));
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::fs::remove_file(&path)?;
    file.write_all(&(pending_input.len() as u32).to_le_bytes())?;
    file.write_all(pending_input)?;
    file.write_all(snapshot)?;
    file.rewind()?;
    Ok(file)
}

/// Read back what [`write_state`] wrote: the pending input goes to
/// `pending_input`, the snapshot is returned.
fn read_state(mut state: File, pending_input: &mut Vec<u8>) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    state.read_exact(&mut len)?;
    pending_input.resize(u32::from_le_bytes(len) as usize, 0);
    state.read_exact(pending_input)?;
    let mut snapshot = Vec::new();
    state.read_to_end(&mut snapshot)?;
    Ok(snapshot)
}
