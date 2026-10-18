//! Exec: a client runs a program on the server's machine without a pane or
//! a shell (`ExecStart`), and gets its stdout and stderr apart
//! (`ExecOutput`) and how it ended (`ExecExit`); it can feed its stdin
//! (`ExecInput`), signal it (`ExecSignal`) or kill it (`ExecCancel`).
//!
//! The program is started from `argv` as it is, with no shell, as the leader
//! of a process group of its own. Nothing of it outlives the exec: when it
//! exits, whatever is left in its group is killed with SIGKILL, and so is
//! the whole group on its time limit, a cancel, the end of its client's
//! connection, and a reload or stop of the server. Execs are not panes and
//! are not handed over in a reload: a client that is still connected gets
//! `ExecExit` with the reason.
//!
//! Output goes by the client's bulk lane ([`ToClient::send_bulk`]): a pipe
//! is read only as fast as the lane has room, so a slow client holds the
//! program up (it blocks writing) rather than the server's memory growing.
//! `ExecExit` goes the same way, after the last of the output, and is always
//! the exec's last message. Stdin the
//! other way is held to [`INPUT_WINDOW`] bytes not yet taken by the
//! program, which the client knows from `ExecInputAck`.
//!
//! The supervisor's actor owns an [`Execs`]: the table of running execs.
//! Each runs in tasks of its own, which tell the actor it ended through the
//! [`Sink`].

use std::collections::HashMap;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tilr_proto::{
    ExecExit, ExecInput, ExecInputAck, ExecOutput, ExecStart, ExecStarted, ServerMessage,
    client_message::Msg, server_message,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle};

use crate::outbox::ToClient;

/// Most bytes in one `ExecOutput`.
const CHUNK: usize = 64 * 1024;
/// Stdin bytes of one exec the client may have sent and the program not yet
/// taken; more is an overrun, and the exec is killed.
pub const INPUT_WINDOW: usize = 4 << 20;
/// Execs one client may have running at once.
const MAX_EXECS_PER_CLIENT: usize = 64;
/// How long after the program's group is gone its output is still waited
/// for: a process that left the group (`setsid`) may hold the pipes open.
/// Only the time spent waiting on the pipe counts, not waiting for room on
/// the client's lane, so a slow client still gets the tail the program left.
const DRAIN: Duration = Duration::from_secs(1);
/// How much is still read of a pipe after the program's group is gone: more
/// than a pipe can hold (Linux's `pipe-max-size` is 1 MiB by default), so
/// all the program left in it, but a process outside the group that keeps
/// writing to a slow client is not followed for ever.
const DRAIN_BYTES: usize = 1 << 20;
/// How long a reload or a stop waits for the execs it killed to report.
const STOP_WAIT: Duration = Duration::from_secs(1);

/// What the tasks hand back to the supervisor's actor.
pub enum Output {
    /// The exec is over; its client has been sent `ExecExit`, if it was
    /// still there.
    Ended { exec_id: u64 },
    /// The client's interactive lane could not take a reply of an exec
    /// (`ExecStarted`, `ExecInputAck`): it is stuck, and is dropped rather
    /// than left waiting for a reply that never comes.
    Stuck { client: u64 },
}

/// Where the tasks' outputs go: to the supervisor's actor.
pub type Sink = Arc<dyn Fn(Output) + Send + Sync>;

/// Why the server killed a program on its own (or as asked).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum End {
    /// Its time limit passed.
    TimedOut,
    /// `ExecCancel`.
    Cancelled,
    /// The client's connection ended.
    ClientGone,
    /// The client sent more stdin than [`INPUT_WINDOW`].
    InputOverrun,
    /// The server reloads.
    Reload,
    /// The server stops.
    Shutdown,
}

impl End {
    /// What the log says.
    fn name(self) -> &'static str {
        match self {
            Self::TimedOut => "timed out",
            Self::Cancelled => "cancelled",
            Self::ClientGone => "client gone",
            Self::InputOverrun => "input overrun",
            Self::Reload => "reload",
            Self::Shutdown => "shutdown",
        }
    }

    /// `ExecExit.reason`: empty for what the exit's flags say.
    fn reason(self) -> &'static str {
        match self {
            Self::TimedOut | Self::Cancelled => "",
            other => other.name(),
        }
    }
}

/// What the actor tells a running exec.
enum Control {
    Signal(i32),
    Kill(End),
}

pub struct Execs {
    sink: Sink,
    runs: HashMap<u64, Run>,
    /// The id the next exec gets; never reused while the server runs.
    next_exec: u64,
}

/// A running exec, as the actor knows it.
struct Run {
    client: u64,
    /// `Hello.client_id` of the client, for the log.
    client_id: String,
    control: mpsc::UnboundedSender<Control>,
    to_client: ToClient,
    /// `None` once stdin is closed, or if it was never asked for.
    stdin: Option<Stdin>,
    task: JoinHandle<()>,
    /// The tasks reading its output and writing its stdin, which a stop
    /// that gives up on the exec ends before it tells the client.
    helpers: Vec<AbortHandle>,
}

/// The way to a program's stdin.
struct Stdin {
    /// Dropping it closes the pipe once what is queued has been written.
    tx: mpsc::UnboundedSender<Bytes>,
    /// Bytes sent and not yet written to the pipe.
    queued: Arc<AtomicUsize>,
}

impl Execs {
    pub fn new(sink: Sink) -> Self {
        Self {
            sink,
            runs: HashMap::new(),
            next_exec: 1,
        }
    }

    /// An Exec message from a client. Returns the answer, if it has one.
    pub fn on_message(
        &mut self,
        client: u64,
        client_id: &str,
        to_client: &ToClient,
        msg: Msg,
    ) -> Option<server_message::Msg> {
        match msg {
            Msg::ExecStart(start) => self
                .start(client, client_id, to_client, start)
                .map(server_message::Msg::ExecStarted),
            Msg::ExecInput(input) => {
                self.on_input(client, input);
                None
            }
            Msg::ExecSignal(signal) => {
                if let Some(run) = self.owned(client, signal.exec_id) {
                    let _ = run.control.send(Control::Signal(signal.signal));
                }
                None
            }
            Msg::ExecCancel(cancel) => {
                if let Some(run) = self.owned(client, cancel.exec_id) {
                    let _ = run.control.send(Control::Kill(End::Cancelled));
                }
                None
            }
            _ => None,
        }
    }

    /// Start a program for a client. Its `ExecStarted` is queued for the
    /// client here, ahead of any output; returned is a refusal.
    fn start(
        &mut self,
        client: u64,
        client_id: &str,
        to_client: &ToClient,
        start: ExecStart,
    ) -> Option<ExecStarted> {
        let refused = |error: String| {
            tracing::info!(
                client,
                client_id,
                request = start.request_id,
                "exec refused: {error}"
            );
            Some(ExecStarted {
                request_id: start.request_id,
                error,
                ..ExecStarted::default()
            })
        };
        let running = self
            .runs
            .values()
            .filter(|run| run.client == client)
            .count();
        if running >= MAX_EXECS_PER_CLIENT {
            return refused(format!("{MAX_EXECS_PER_CLIENT} execs running already"));
        }
        let mut child = match spawn(&start) {
            Ok(child) => child,
            Err(err) => return refused(err),
        };
        let Some(pid) = child.id() else {
            return refused("the program is gone".into());
        };
        let exec_id = self.next_exec;
        self.next_exec += 1;
        let started = encode(server_message::Msg::ExecStarted(ExecStarted {
            request_id: start.request_id,
            exec_id,
            pid,
            error: String::new(),
        }));
        if to_client.try_send(started.into()).is_err() {
            tracing::debug!(client, "exec dropped: the client is not taking packets");
            kill_group(pid, libc::SIGKILL);
            (self.sink)(Output::Stuck { client });
            return None;
        }
        tracing::info!(
            client,
            client_id,
            exec = exec_id,
            pid,
            program = start.argv[0],
            args = start.argv.len() - 1,
            "exec started"
        );

        let (over_tx, over) = watch::channel(false);
        let output = tokio::spawn({
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let to_client = to_client.clone();
            async move {
                tokio::join!(
                    pump(exec_id, 1, stdout, to_client.clone(), over.clone()),
                    pump(exec_id, 2, stderr, to_client, over),
                );
            }
        });
        let (stdin, feed) = match child.stdin.take() {
            Some(pipe) => {
                let (tx, rx) = mpsc::unbounded_channel();
                let queued = Arc::new(AtomicUsize::new(0));
                let feed = tokio::spawn(feed(
                    exec_id,
                    client,
                    pipe,
                    rx,
                    Arc::clone(&queued),
                    to_client.clone(),
                    Arc::clone(&self.sink),
                ));
                (Some(Stdin { tx, queued }), Some(feed))
            }
            None => (None, None),
        };
        let mut helpers = vec![output.abort_handle()];
        helpers.extend(feed.as_ref().map(JoinHandle::abort_handle));
        let (control, control_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(Running {
            exec_id,
            client,
            client_id: client_id.to_owned(),
            child,
            pid,
            limit: (start.timeout_ms > 0).then(|| Duration::from_millis(start.timeout_ms.into())),
            control: control_rx,
            output,
            over: over_tx,
            feed,
            to_client: to_client.clone(),
            sink: Arc::clone(&self.sink),
        }));
        self.runs.insert(
            exec_id,
            Run {
                client,
                client_id: client_id.to_owned(),
                control,
                to_client: to_client.clone(),
                stdin,
                task,
                helpers,
            },
        );
        None
    }

    /// Stdin for a program: queued for it, or the end of it.
    fn on_input(&mut self, client: u64, input: ExecInput) {
        let Some(run) = self
            .runs
            .get_mut(&input.exec_id)
            .filter(|run| run.client == client)
        else {
            tracing::debug!(
                client,
                exec = input.exec_id,
                "input for an exec not the client's"
            );
            return;
        };
        let Some(stdin) = &run.stdin else {
            tracing::debug!(client, exec = input.exec_id, "input after the end of stdin");
            return;
        };
        if !input.data.is_empty() {
            let len = input.data.len();
            if stdin.queued.fetch_add(len, Ordering::Relaxed) + len > INPUT_WINDOW {
                tracing::warn!(
                    client,
                    exec = input.exec_id,
                    "exec killed: the client sent stdin {INPUT_WINDOW} bytes ahead of the program"
                );
                run.stdin = None;
                let _ = run.control.send(Control::Kill(End::InputOverrun));
                return;
            }
            let _ = stdin.tx.send(input.data);
        }
        if input.eof {
            // The pipe closes once what is queued is written.
            run.stdin = None;
        }
    }

    /// The exec's tasks are done.
    pub fn ended(&mut self, exec_id: u64) {
        self.runs.remove(&exec_id);
    }

    /// The client is gone: its programs are killed.
    pub fn drop_client(&mut self, client: u64) {
        for run in self.runs.values().filter(|run| run.client == client) {
            let _ = run.control.send(Control::Kill(End::ClientGone));
        }
    }

    /// Kill every program, for a reload or a stop, and wait a little for
    /// each to report to its client. One that does not in time is given
    /// up: its client is told without the status. `false` if none ran.
    pub async fn stop_all(&mut self, why: End) -> bool {
        if self.runs.is_empty() {
            return false;
        }
        tracing::info!(
            execs = self.runs.len(),
            reason = why.reason(),
            "ending the execs"
        );
        for run in self.runs.values() {
            let _ = run.control.send(Control::Kill(why));
        }
        let deadline = tokio::time::Instant::now() + STOP_WAIT;
        for (exec_id, mut run) in self.runs.drain() {
            if tokio::time::timeout_at(deadline, &mut run.task)
                .await
                .is_ok()
            {
                continue;
            }
            // Its readers first, so that no output of it follows the
            // stand-in below.
            run.task.abort();
            for helper in &run.helpers {
                helper.abort();
            }
            tracing::warn!(
                client = run.client,
                client_id = run.client_id,
                exec = exec_id,
                reason = why.reason(),
                "exec ended without its status"
            );
            let exit = encode(server_message::Msg::ExecExit(ExecExit {
                exec_id,
                reason: why.reason().into(),
                ..ExecExit::default()
            }));
            // Behind the output already queued on the bulk lane, without
            // waiting for room: one small packet over the bound.
            let _ = run.to_client.queue_bulk_now(exit);
        }
        true
    }

    fn owned(&self, client: u64, exec_id: u64) -> Option<&Run> {
        let run = self.runs.get(&exec_id).filter(|run| run.client == client);
        if run.is_none() {
            tracing::debug!(client, exec = exec_id, "exec not the client's");
        }
        run
    }
}

/// Start the program of an `ExecStart`, or say why it cannot be.
fn spawn(start: &ExecStart) -> Result<Child, String> {
    let Some(program) = start.argv.first() else {
        return Err("no program".into());
    };
    let cwd = if start.cwd.is_empty() {
        std::env::home_dir().unwrap_or_else(|| "/".into())
    } else {
        start.cwd.clone().into()
    };
    if !Path::new(&cwd).is_dir() {
        return Err(format!("no such directory: {}", cwd.display()));
    }
    let mut cmd = Command::new(program);
    cmd.args(&start.argv[1..])
        .current_dir(&cwd)
        .stdin(if start.stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The program leads a group of its own, which is what is signalled
        // and killed: its children go with it.
        .process_group(0)
        .kill_on_drop(true);
    for entry in &start.env {
        match entry.split_once('=') {
            Some((name, value)) if !name.is_empty() => {
                cmd.env(name, value);
            }
            _ => return Err(format!("not NAME=value: {entry:?}")),
        }
    }
    // Should the server die without killing it, the program goes too
    // (its children are left to the group kill).
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn().map_err(|err| format!("{program}: {err}"))
}

/// Signal a process group.
fn kill_group(pgid: u32, signal: i32) {
    // SAFETY: kill has no memory-safety preconditions.
    unsafe {
        libc::kill(-(pgid as libc::pid_t), signal);
    }
}

/// Everything a running exec's main task holds.
struct Running {
    exec_id: u64,
    client: u64,
    client_id: String,
    child: Child,
    /// Of the program, and of its process group.
    pid: u32,
    limit: Option<Duration>,
    control: mpsc::UnboundedReceiver<Control>,
    /// The task reading stdout and stderr.
    output: JoinHandle<()>,
    /// Told when the program's group is gone: the readers then give up a
    /// pipe that stays open longer than [`DRAIN`] allows.
    over: watch::Sender<bool>,
    /// The task writing stdin.
    feed: Option<JoinHandle<()>>,
    to_client: ToClient,
    sink: Sink,
}

/// Wait for the program to end, signalling and killing it as asked or as
/// its time limit says, then for its output to end, and tell the client.
async fn run(mut r: Running) {
    let started = Instant::now();
    let deadline = r.limit.map(|limit| tokio::time::Instant::now() + limit);
    let mut end = None;
    let mut controlled = true;
    // Before the first look, so that no exit can slip between the two.
    let mut sigchld = signal(SignalKind::child()).ok();
    loop {
        tokio::select! {
            () = exited(r.pid, &mut sigchld) => break,
            () = sleep_until(deadline), if end.is_none() => {
                end = Some(End::TimedOut);
                kill_group(r.pid, libc::SIGKILL);
            }
            control = r.control.recv(), if controlled => match control {
                Some(Control::Signal(signal)) => {
                    if end.is_none() {
                        kill_group(r.pid, signal);
                    }
                }
                Some(Control::Kill(why)) => {
                    end.get_or_insert(why);
                    kill_group(r.pid, libc::SIGKILL);
                }
                // The actor is gone: so is the server.
                None => {
                    controlled = false;
                    end.get_or_insert(End::Shutdown);
                    kill_group(r.pid, libc::SIGKILL);
                }
            },
        }
    }
    // What the program left behind in its group goes with it. It has exited
    // but is not reaped yet: its zombie still holds its pid, the group's id,
    // so no other group can have taken it. No signal is sent after the reap.
    kill_group(r.pid, libc::SIGKILL);
    let status = r.child.wait().await;
    let _ = r.over.send(true);
    // The output is read to its end, which a process that left the group
    // may hold back a while; a cancel, the time limit or the client going
    // cut that short at once.
    loop {
        tokio::select! {
            _ = &mut r.output => break,
            () = sleep_until(deadline), if end.is_none() => {
                end = Some(End::TimedOut);
                break;
            }
            control = r.control.recv(), if controlled => match control {
                Some(Control::Signal(_)) => {}
                Some(Control::Kill(why)) => {
                    end.get_or_insert(why);
                    break;
                }
                None => {
                    end.get_or_insert(End::Shutdown);
                    break;
                }
            },
        }
    }
    r.output.abort();
    if let Some(feed) = r.feed {
        feed.abort();
    }
    let status = status.ok();
    let code = status.and_then(|status| status.code()).unwrap_or(0);
    let signal = status.and_then(|status| status.signal()).unwrap_or(0);
    let reason = end.map_or("", End::reason);
    tracing::info!(
        client = r.client,
        client_id = r.client_id,
        exec = r.exec_id,
        code,
        signal,
        killed = end.map_or("", End::name),
        ms = started.elapsed().as_millis() as u64,
        "exec ended"
    );
    let exit = encode(server_message::Msg::ExecExit(ExecExit {
        exec_id: r.exec_id,
        code,
        signal,
        timed_out: end == Some(End::TimedOut),
        cancelled: end == Some(End::Cancelled),
        reason: reason.into(),
    }));
    // By the bulk lane: after the last of the output.
    let _ = r.to_client.send_bulk(exit).await;
    (r.sink)(Output::Ended { exec_id: r.exec_id });
}

/// Once the program has exited, without reaping it: its zombie keeps its pid
/// (and so its group's id) from being reused until [`Child::wait`].
/// Woken by SIGCHLD, and once a second should a signal be missed.
async fn exited(pid: u32, sigchld: &mut Option<Signal>) {
    loop {
        if has_exited(pid) {
            return;
        }
        let wake = tokio::time::sleep(Duration::from_secs(1));
        match sigchld {
            Some(sigchld) => {
                tokio::select! {
                    _ = sigchld.recv() => {}
                    () = wake => {}
                }
            }
            None => wake.await,
        }
    }
}

/// Whether the child `pid` has exited, leaving it a zombie (`WNOWAIT`).
fn has_exited(pid: u32) -> bool {
    // SAFETY: an all-zero siginfo_t is a valid value of it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid siginfo_t to write to.
    let found = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &raw mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if found == -1 {
        // Not our child any more (reaped): nothing to wait for. EINTR: look
        // again on the next wake.
        return std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR);
    }
    // SAFETY: waitid filled in `info`, or left it zeroed if nothing exited.
    unsafe { info.si_pid() != 0 }
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Send what the program writes to one of its pipes to the client, as fast
/// as the client's bulk lane takes it, until the pipe ends or the client is
/// gone. Once the program's group is gone (`over`), the pipe is waited on for
/// at most [`DRAIN`] in all and read for at most [`DRAIN_BYTES`] more: a
/// process that left the group may hold it open, or keep writing to it.
async fn pump(
    exec_id: u64,
    stream: u32,
    pipe: Option<impl AsyncRead + Unpin + Send>,
    to_client: ToClient,
    mut over: watch::Receiver<bool>,
) {
    let Some(mut pipe) = pipe else {
        return;
    };
    let mut buf = vec![0u8; CHUNK];
    // What is left of the drain once the group is gone: time, bytes.
    let mut drain = None;
    loop {
        if drain.is_none() && *over.borrow() {
            drain = Some((DRAIN, DRAIN_BYTES));
        }
        let read = match &mut drain {
            None => tokio::select! {
                read = pipe.read(&mut buf) => read,
                // Read again, now against the drain's limits.
                () = group_gone(&mut over) => continue,
            },
            Some((time, bytes)) => {
                let waited = tokio::time::Instant::now();
                let read = if time.is_zero() || *bytes == 0 {
                    None
                } else {
                    let len = buf.len().min(*bytes);
                    tokio::time::timeout(*time, pipe.read(&mut buf[..len]))
                        .await
                        .ok()
                };
                *time = time.saturating_sub(waited.elapsed());
                let Some(read) = read else {
                    tracing::debug!(
                        exec = exec_id,
                        stream,
                        "output still coming after the program: left"
                    );
                    return;
                };
                if let Ok(n) = read {
                    *bytes -= n;
                }
                read
            }
        };
        let n = match read {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let packet = encode(server_message::Msg::ExecOutput(ExecOutput {
            exec_id,
            stream,
            data: Bytes::copy_from_slice(&buf[..n]),
        }));
        if to_client.send_bulk(packet).await.is_err() {
            return;
        }
    }
}

/// Once the program's group is gone; never if the exec's task is.
async fn group_gone(over: &mut watch::Receiver<bool>) {
    if over.wait_for(|gone| *gone).await.is_err() {
        std::future::pending().await
    }
}

/// Write what the client sends to the program's stdin, telling the client
/// how much it has taken, and close it when the client says so (`from`
/// ends). A program that closed its stdin has the rest thrown away, still
/// acknowledged, so the client is not held up.
async fn feed(
    exec_id: u64,
    client: u64,
    mut pipe: ChildStdin,
    mut from: mpsc::UnboundedReceiver<Bytes>,
    queued: Arc<AtomicUsize>,
    to_client: ToClient,
    sink: Sink,
) {
    let mut taken = 0u64;
    let mut open = true;
    while let Some(data) = from.recv().await {
        if open && pipe.write_all(&data).await.is_err() {
            open = false;
        }
        queued.fetch_sub(data.len(), Ordering::Relaxed);
        taken += data.len() as u64;
        // Unless more is queued already, which acknowledges it too.
        if from.is_empty() {
            let ack = encode(server_message::Msg::ExecInputAck(ExecInputAck {
                exec_id,
                taken,
            }));
            if to_client.try_send(ack.into()).is_err() {
                // A client left without its last ack would wait for room
                // for ever.
                sink(Output::Stuck { client });
                return;
            }
        }
    }
    // Dropping the pipe closes it: the program reads end of file.
}

fn encode(msg: server_message::Msg) -> Bytes {
    tilr_proto::encode(&ServerMessage::from(msg))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[tokio::test]
    async fn the_group_is_killed_while_the_program_is_a_zombie() {
        // A program that leaves a child behind in its group and exits.
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & exit 3"])
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let mut sigchld = signal(SignalKind::child()).ok();
        tokio::time::timeout(Duration::from_secs(5), exited(pid, &mut sigchld))
            .await
            .expect("the program exits");
        // Exited, not reaped: its pid, the group's id, is still its own.
        let stat = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&stat.stdout);
        assert!(stat.trim().starts_with('Z'), "state {stat:?}");
        kill_group(pid, libc::SIGKILL);
        assert_eq!(child.wait().await.unwrap().code(), Some(3));
        // The child it left went with the group.
        let mut gone = false;
        for _ in 0..50 {
            // SAFETY: kill has no memory-safety preconditions.
            if unsafe { libc::kill(-(pid as libc::pid_t), 0) } == -1 {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone, "the group outlived the kill");
    }

    #[tokio::test]
    async fn a_reply_the_lane_cannot_take_drops_the_client() {
        let stuck = Arc::new(Mutex::new(Vec::new()));
        let mut execs = Execs::new({
            let stuck = Arc::clone(&stuck);
            Arc::new(move |output| {
                if let Output::Stuck { client } = output {
                    stuck.lock().unwrap().push(client);
                }
            })
        });
        let (to_client, _outbox) = crate::outbox::channel();
        while to_client
            .try_send(Bytes::from_static(b"frame").into())
            .is_ok()
        {}
        let start = ExecStart {
            request_id: 1,
            argv: vec!["sleep".into(), "30".into()],
            ..ExecStart::default()
        };
        let answer = execs.on_message(7, "", &to_client, Msg::ExecStart(start));
        assert!(answer.is_none(), "{answer:?}");
        assert_eq!(*stuck.lock().unwrap(), [7], "the client is reported stuck");
        assert!(execs.runs.is_empty(), "no exec is left running for it");
    }
}
