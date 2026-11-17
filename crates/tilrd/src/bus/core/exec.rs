//! `exec.*` (T2), feature `exec`: `exec.run` (a bulk stream), `exec.input`,
//! `exec.signal`.
//!
//! Owner: `bus-core-services`.
//!
//! A program runs in `crate::exec::Execs`, as one of `tilr.v1`'s: its own
//! process group, killed with its group when it ends, at its time limit, on
//! cancel, when its caller's connection ends, at a reload or a stop. What it
//! does reaches the stream through a [`BusReport`]: a short queue to the
//! stream's handler, which sends each chunk within the reader's credit, so a
//! reader that credits slowly holds the program back. `server.update` and
//! `server.command` stream through [`stream_exec`] too.

use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use prost::Message;
use tilr_bus::bus::Lane;
use tilr_bus::exec::{
    ExecExit, ExecOutput, ExecStarted, InputReply, InputRequest, RunChunk, RunRequest, SignalReply,
    SignalRequest, run_chunk,
};
use tilr_bus::{BusError, codes, methods};
use tokio::sync::{mpsc, oneshot};

use super::fs::{room, split};
use super::json;
use super::server::{asked, conn_of, json_message, limit, on_connection_end, queue, unary};
use super::{Core, Registry, SupervisorHandle};
use crate::bus::identity::CallCtx;
use crate::bus::streams::{Chunk, NativeMethod, StreamOut};
use crate::exec::{Exit, Pending, Program, Refusal, Report};
use crate::supervisor::Supervisor;

json_message!(RunRequest {
    argv: strings "argv",
    cwd: string "cwd",
    env: strings "env",
    stdin: bool "stdin",
    timeout_ms: u32 "timeoutMs",
});
json_message!(InputRequest {
    exec_id: u64 "execId",
    data: bytes "data",
    eof: bool "eof",
});
json_message!(InputReply {
    exec_id: u64 "execId",
    taken: u64 "taken",
});
json_message!(SignalRequest {
    exec_id: u64 "execId",
    signal: i32 "signal",
});
json_message!(SignalReply {});

/// Add `exec.*` when `core.features.exec`.
pub fn register(registry: &mut Registry, core: &Core) {
    if !core.features.exec {
        return;
    }
    let runner = core.clone();
    registry.add(NativeMethod::stream(
        methods::EXEC_RUN.name,
        Lane::Bulk,
        move |ctx, payload, out| {
            let core = runner.clone();
            async move {
                let request = match json::decode::<RunRequest>(&ctx, &payload) {
                    Ok(request) => request,
                    Err(err) => return out.fail(err),
                };
                let program = Program {
                    argv: request.argv,
                    cwd: request.cwd,
                    env: request.env,
                    stdin: request.stdin,
                    timeout_ms: request.timeout_ms,
                };
                stream_exec(core, ctx, program, out).await;
            }
        },
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::EXEC_INPUT.name,
        move |ctx: &CallCtx, request: InputRequest| {
            let conn = conn_of(ctx);
            let exec_id = request.exec_id;
            let (tx, taken) = oneshot::channel();
            // Queued now: the stdin of one caller keeps its order.
            let queued = queue(&supervisor, move |s| {
                execs(s, |execs| {
                    execs.input(conn, exec_id, request.data, request.eof, Some(tx))
                })
            });
            async move {
                asked(queued?).await??;
                let taken = taken
                    .await
                    .map_err(|_| BusError::unavailable(format!("exec {exec_id} ended")))?;
                Ok(InputReply { exec_id, taken })
            }
        },
    ));
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::EXEC_SIGNAL.name,
        move |ctx: &CallCtx, request: SignalRequest| {
            let conn = conn_of(ctx);
            let signalled = queue(&supervisor, move |s| {
                execs(s, |execs| {
                    execs.signal(conn, request.exec_id, request.signal)
                })
            });
            async move {
                asked(signalled?).await??;
                Ok(SignalReply {})
            }
        },
    ));
}

/// Run `f` on the server's execs; its `Err` is `not_found` for an exec that
/// is not the caller's, `busy` at [`crate::exec::MAX_EXECS_PER_CLIENT`],
/// `bad_request` else.
fn execs<T>(
    s: &mut Supervisor,
    f: impl FnOnce(&mut crate::exec::Execs) -> Result<T, Refusal>,
) -> Result<T, BusError> {
    let execs = s
        .execs
        .as_mut()
        .ok_or_else(|| BusError::unavailable("tilrd is shutting down"))?;
    f(execs).map_err(|refusal| match refusal {
        Refusal::NoExec(why) => BusError::new(codes::NOT_FOUND, why),
        Refusal::Limit(why) => limit(why),
        Refusal::Refused(why) => BusError::bad_request(why),
    })
}

/// The caller's connection ended: its programs are killed.
fn forget_connection(s: &mut Supervisor, conn: u64) {
    if let Some(execs) = &mut s.execs {
        execs.drop_client(conn);
    }
}

/// What an exec does, on its way to the stream's handler.
enum Item {
    Started { exec_id: u64, pid: u32 },
    Output(ExecOutput),
    Exit(Exit),
}

/// The [`Report`] of an exec on the bus: a short queue to [`stream_exec`].
struct BusReport {
    items: mpsc::Sender<Item>,
    /// The end of an exec a stop gave up on: not behind its output.
    now: mpsc::UnboundedSender<Exit>,
}

impl Report for BusReport {
    fn started(&self, exec_id: u64, pid: u32) -> bool {
        // The queue is empty yet: this goes.
        self.items.try_send(Item::Started { exec_id, pid }).is_ok()
    }

    fn output(&self, exec_id: u64, stream: u32, data: Bytes) -> Pending<'_, Result<(), ()>> {
        Box::pin(async move {
            self.items
                .send(Item::Output(ExecOutput {
                    exec_id,
                    stream,
                    data,
                }))
                .await
                .map_err(|_| ())
        })
    }

    fn input_taken(&self, _exec_id: u64, _taken: u64) -> bool {
        // Each `exec.input` is answered by its own reply.
        true
    }

    fn exit(&self, exit: Exit) -> Pending<'_, ()> {
        Box::pin(async move {
            let _ = self.items.send(Item::Exit(exit)).await;
        })
    }

    fn exit_now(&self, exit: Exit) {
        let _ = self.now.send(exit);
    }
}

fn exec_exit(exit: Exit) -> ExecExit {
    ExecExit {
        exec_id: exit.exec_id,
        code: exit.code,
        signal: exit.signal,
        timed_out: exit.timed_out,
        cancelled: exit.cancelled,
        reason: exit.reason,
    }
}

fn run_chunk(chunk: run_chunk::Chunk) -> Chunk {
    Chunk::new(RunChunk { chunk: Some(chunk) }.encode_to_vec())
}

/// Cancels the exec when the stream's handler goes before its end (the
/// broker aborts it when the caller cancels or its connection ends). Made
/// before the start is queued: the cancel is queued behind the start, so a
/// handler that goes at any point takes the program with it.
struct CancelOnDrop {
    supervisor: SupervisorHandle,
    conn: u64,
    /// The exec's id, set by the start on the supervisor's task.
    exec_id: Arc<OnceLock<u64>>,
    armed: bool,
}

impl CancelOnDrop {
    /// Kill the exec as cancelled (once it started), now.
    fn cancel(&mut self) {
        if std::mem::take(&mut self.armed) {
            let (conn, exec_id) = (self.conn, Arc::clone(&self.exec_id));
            let _ = self.supervisor.with(move |s| {
                if let (Some(execs), Some(&exec_id)) = (&mut s.execs, exec_id.get()) {
                    execs.cancel(conn, exec_id);
                }
            });
        }
    }

    /// The exec ended: nothing to cancel.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The failpoint between the start of a program and the stream's handler
/// taking up its answer.
#[cfg(feature = "failpoints")]
const STARTED: &str = "bus-exec-started";

/// Run `program` for the caller and stream it: `started`, the `output`
/// chunks (split to the stream's window), `End` with the `ExecExit`; a program
/// that did not start is an error `spawn_failed` (`busy` at the limit of a
/// connection's programs). Cancelling the stream, or a stream whose reader is
/// gone, kills the program's group.
pub(super) async fn stream_exec(core: Core, ctx: CallCtx, program: Program, out: StreamOut) {
    let conn = conn_of(&ctx);
    let client_id = ctx.principal.client_id.clone();
    let (items_tx, mut items) = mpsc::channel(2);
    let (now_tx, mut now) = mpsc::unbounded_channel();
    let report = Arc::new(BusReport {
        items: items_tx,
        now: now_tx,
    });
    let mut guard = CancelOnDrop {
        supervisor: core.supervisor.clone(),
        conn,
        exec_id: Arc::new(OnceLock::new()),
        armed: true,
    };
    let exec_id = Arc::clone(&guard.exec_id);
    let started = queue(&core.supervisor, move |s| {
        if !on_connection_end(s, "exec", conn, forget_connection) {
            return Err(BusError::unavailable("the connection ended"));
        }
        let started = execs(s, |execs| execs.start(conn, &client_id, program, report));
        match &started {
            Ok(id) => {
                let _ = exec_id.set(*id);
            }
            Err(err) => tracing::info!(conn, client_id, "exec refused: {}", err.message),
        }
        started
    });
    let started = match started {
        Ok(rx) => asked(rx).await.and_then(|started| started),
        Err(err) => Err(err),
    };
    #[cfg(feature = "failpoints")]
    if crate::fail::armed(STARTED) {
        crate::fail::point_async(STARTED.into()).await;
    }
    if let Err(err) = started {
        guard.disarm();
        let err = if err.code == codes::BAD_REQUEST {
            BusError::new(codes::SPAWN_FAILED, err.message)
        } else {
            err
        };
        return out.fail(err);
    }
    let output_room = room(
        out.window(),
        RunChunk {
            chunk: Some(run_chunk::Chunk::Output(ExecOutput {
                exec_id: u64::MAX,
                stream: 2,
                data: Bytes::new(),
            })),
        }
        .encoded_len(),
    );
    let mut cancelled = false;
    let mut now_open = true;
    loop {
        tokio::select! {
            biased;
            exit = now.recv(), if now_open => match exit {
                Some(exit) => {
                    guard.disarm();
                    return out.end(exec_exit(exit).encode_to_vec().into());
                }
                None => now_open = false,
            },
            () = out.cancelled().cancelled(), if !cancelled => {
                // The program is killed; its end is waited for below.
                cancelled = true;
                guard.cancel();
            }
            item = items.recv() => match item {
                Some(Item::Started { exec_id, pid }) => {
                    if cancelled {
                        continue;
                    }
                    let started = run_chunk::Chunk::Started(ExecStarted { exec_id, pid });
                    if out.send(run_chunk(started)).await.is_err() {
                        // Nobody reads it: the program goes.
                        cancelled = true;
                        guard.cancel();
                    }
                }
                Some(Item::Output(output)) => {
                    if cancelled {
                        continue;
                    }
                    for data in split(output.data, output_room) {
                        let chunk = run_chunk::Chunk::Output(ExecOutput {
                            exec_id: output.exec_id,
                            stream: output.stream,
                            data,
                        });
                        if out.send(run_chunk(chunk)).await.is_err() {
                            cancelled = true;
                            guard.cancel();
                            break;
                        }
                    }
                }
                Some(Item::Exit(exit)) => {
                    guard.disarm();
                    return out.end(exec_exit(exit).encode_to_vec().into());
                }
                None => {
                    guard.disarm();
                    return out.fail(BusError::unavailable("the program's end was not told"));
                }
            },
        }
    }
}
