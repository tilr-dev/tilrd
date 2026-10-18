# Design

## Context

At 1163abd an exec is three tasks: `run()` (waits for the program, its time limit and the client's control), two `pump()` readers (stdout, stderr) joined in one output task, and `feed()` (stdin). After the program exits, `run()` kills the group, tells the readers by a `watch` that the group is gone (`over`, now + 1 s) and awaits them, then sends `ExecExit` on the bulk lane.

## Decisions

### D1. The drain is bounded by time spent waiting on the pipe
`pump()` keeps a budget of 1 s that starts when `over` is set and is spent only while the reader waits for or reads the pipe (not while it waits in `send_bulk` for room on the client's lane). The loop is no longer `biased`: before each read it checks the budget and stops when it is spent. A process outside the group that writes without pause is cut off after 1 s; the tail the program itself left in the pipe still reaches a slow client whole, as today.

`run()` awaits the readers in a `select!` with its control channel and its deadline: `ExecCancel`, the client going and the time limit abort the readers at once and end the exec with `cancelled` / `timed_out` as usual.

Rejected: a hard 1 s cap on the whole drain (a slow client would lose the end of a program that already exited); keeping `biased` and polling the deadline every N reads (still unbounded under a fast writer between polls).

### D2. Stand-in ExecExit after the readers stop
`Run` keeps abort handles of the output and stdin tasks. When `stop_all` gives up on an exec it aborts those first, then queues the stand-in `ExecExit` on the bulk lane with a new `ToClient::queue_bulk_now` that does not wait for byte credit (one packet of a few bytes over the 4 MiB bound). The connection then closes with a flush (reload) as today, writing the bulk lane after the interactive one, so the stand-in comes after the output.

### D3. Replies that cannot be queued
`ExecStart` with exec off goes to `Execs`-less handling that answers `ExecStarted { request_id, exec_id: 0, error: "exec is turned off" }`. Per the protocol-v3 rule a client checks `"exec"` before relying on Exec; the answer only spares a client that did not a hang.

`ExecStarted` and `ExecInputAck` are sent with `try_send` because the exec tasks cannot wait on the interactive lane. A failure now reports `Output::Stuck { client }` to the supervisor, which drops the client as too slow (close code 4001, log "client too slow"), the rule frames already follow; the exec is killed with the client's other execs.

### D4. Kill the group before the reap
On exit the program is first observed without reaping it (`waitid(P_PID, pid, WEXITED | WNOWAIT)` on a blocking thread, or the SIGCHLD stream plus `waitid` with `WNOHANG | WNOWAIT`), the group is killed while the zombie leader still reserves the id, and only then reaped (`Child::wait`). Signals sent for `ExecSignal`, the time limit and cancel are sent only before that point.

## Backpressure and memory

Output stays on the bulk lane (4 MiB per client, 1 MiB unread in the network). D2 adds one small packet beyond the bound per exec given up, at most 64 per client. D1 adds no buffering.

## Reload and reconnect

Execs do not cross a reload: `stop_all` (D2) runs before the exec of the successor, as now. A client that reconnects starts with no execs.
