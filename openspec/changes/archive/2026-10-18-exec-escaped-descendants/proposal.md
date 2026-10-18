# Proposal: An exec always ends, ExecExit is its last message, and its replies are never lost

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found (findings #6, #15, #16, #18). Re-checked against the code at 1163abd (`crates/tilrd/src/exec.rs` is unchanged since 13f9522):

- #6 as reported (the deadline is never polled) does not hold: `run()` waits for the program, its time limit and `ExecCancel` side by side (`exec.rs:483-507`), and a pipe held open by a silent process outside the group is given up 1 s after the group is gone (`given_up`, `exec.rs:513`, `591-596`). What does hold, by reading the code (untested): the reader's `select!` is `biased` toward the pipe (`exec.rs:568-579`), so a process outside the group that keeps *writing* (`setsid sh -c 'while :; do echo; done' &`) keeps the read branch ready and the 1 s give-up is never reached; `run()` meanwhile awaits the readers without looking at `ExecCancel` (`exec.rs:514`). No `ExecExit` comes, the exec's slot is never freed (it counts toward the 64 per client) and cancel does nothing.
- #15: when a reload or stop gives up waiting for an exec (`stop_all`, `exec.rs:350-386`) it aborts only the exec's main task; the reader tasks it owned keep running (a dropped `JoinHandle` does not abort), so `ExecOutput` can follow the stand-in `ExecExit`, which is moreover queued on the interactive lane (`exec.rs:383`) ahead of output still queued on the bulk lane.
- #16: `ExecStart` with exec off is dropped with a debug log (`supervisor.rs:2193-2195`); `ExecStarted` that does not fit the interactive lane kills the program and answers nothing (`exec.rs:226-230`); an `ExecInputAck` that does not fit is dropped (`exec.rs:619-623`), and if it was the last one the client waits for room for ever.
- #18: after the program is reaped, its group is signalled once more (`exec.rs:512`); in that instant the pid, no longer reserved by a zombie leader, may be taken by a new process group (a new pane's shell calls `setsid`), which then gets the SIGKILL.

## What Changes

- A pipe still open after the program's group is gone is read for at most 1 s of waiting on the pipe (time spent waiting for room on the client's bulk lane does not count), however much it writes; then the exec ends. `ExecCancel` and the time limit cut that drain short at once (design D1).
- `ExecExit` is always the exec's last message: when a reload or stop gives up on an exec, its readers are stopped first and the stand-in `ExecExit` goes on the bulk lane behind the output already queued (design D2).
- `ExecStart` with exec off is answered with `ExecStarted { error: "exec is turned off" }` instead of nothing (design D3).
- `ExecStarted` and `ExecInputAck` are never silently lost: a client whose interactive lane cannot take one is dropped as too slow (close code 4001), as for frames (design D3).
- The group is killed while the program is a zombie, before it is reaped, so its id cannot have been reused (design D4).

## Capabilities

### Modified Capabilities
- `exec`: see `specs/exec/spec.md`.

## Non-goals

- Tracking processes that leave the group (they are outside the exec by design; only their hold on the pipes is bounded).
- Changing the 4 MiB stdin window, the 64-exec limit or the 1 s stop wait.
- A feature flag or protocol change: `ExecStarted.error` already exists.

## Impact

`crates/tilrd/src/exec.rs` (reader loop, `run()`, `stop_all`, reap order), `crates/tilrd/src/outbox.rs` (a small bulk packet queued without waiting for room), `crates/tilrd/src/supervisor.rs` (exec off answer; dropping a client whose lane cannot take an exec reply). Not breaking: a client that already checks `HelloAck.features` never sees the new refusal. Nothing crosses a reload (execs end at a reload, unchanged); an update from the previous release changes nothing on disk.
