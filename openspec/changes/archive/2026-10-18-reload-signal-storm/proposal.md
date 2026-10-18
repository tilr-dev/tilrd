# Proposal: no thread takes a held signal while the supervisor execs

## Why

Itsaplan TILR-121 (phase 1, bug). `reload::a_storm_of_reload_signals_costs_no_shell` failed in a full macOS run at `crates/tilrd/tests/reload.rs:1044` (`server.is_running()`): the supervisor vanished right after a `reloading` log line with no error of its own, and its pane logged `send status to supervisor: Broken pipe`. Checked at c888b6d:

- The reload holds SIGUSR1, SIGHUP, SIGTERM and SIGINT across the exec by blocking them in `pre_exec` on the thread that execs (`reload::hold_signals`, `crates/tilrd/src/reload.rs:285`, called at `crates/tilrd/src/supervisor.rs:2048`). The mask is per thread: every other thread of the supervisor (the tokio workers, the blocking pool, the file watchers' threads) still takes them, with tokio's handlers (`supervisor.rs:500`).
- On macOS a signal handed to one of those threads while `execve` replaces the image ends the process. Reproduced without tilrd: a C program with four threads that blocks SIGUSR1 on the exec'ing thread only and execs itself in a loop dies within seconds of a SIGUSR1 flood (SIGILL; the handler runs on a thread whose image is gone), and survives 20000 execs when every thread has it blocked. With tilrd, SIGUSR1 sent every 0.2 ms killed the server in 3 of 5 runs of 10 s (exit by SIGILL, the last log line `reloading exe=...`). Linux (`scripts/linux.sh`) survives the same flood: `execve` ends the other threads before it resets the handlers.
- On both systems a signal taken by another thread in that window is lost: tokio's handler queues the event in the image about to be replaced. A SIGTERM there is dropped and the server keeps running (`reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor`, failpoint `reload-exec=sleep:1500`, fails at c888b6d on macOS).

## What Changes

- A server blocks the four signals on its main thread before it starts any other, so every thread it starts has them blocked, and lets them in on one thread of their own (`signals`), where their handlers run.
- Before the exec, the supervisor has that thread block them too (and waits for it), so no thread of the image takes one: what comes waits, pending, for the successor, which records and handles it as it already does. If the exec fails, the thread lets them in again.
- A child the supervisor starts (pane processes, the successor checks, `exec` programs, `cloudflared`) is started with the four let in on the spawning thread, so it inherits none of them blocked; no child is started while an exec is under way.
- A failpoint right before the exec (`reload-exec`), for the test.

## Capabilities

### Modified Capabilities
- `reload`: see `specs/reload/spec.md` (the held signals include those arriving during the exec itself).

## Non-goals

- Pane processes: a pane process's own exec (`crates/tilrd/src/pane.rs:638`) does not hold signals; nothing signals a pane process but the supervisor and a service manager stopping the whole group.
- What the four signals do once the server runs.
- The SIGPIPE disposition that the standard library sets to the default for an exec and does not restore when the exec fails (noted separately).

## Impact

`crates/tilrd/src/reload.rs` (the signal thread, `hold_everywhere` / `release_everywhere`, `spawn_unblocked`), `crates/tilrd/src/main.rs` (started for the server command after `record_signals`), `crates/tilrd/src/supervisor.rs` (exec path, failpoint, pane spawn), `crates/tilrd/src/exec.rs`, `crates/tilrd/src/cloud.rs`, `crates/tilrd/tests/reload.rs`, README reload section, USE_CASES.md. No protocol change; nothing new crosses a reload (the held signals are the mask and the pending set, as before); `state-formats` unchanged. A reload from the previous release into this one: the predecessor still execs with only its exec'ing thread holding them (a signal in that one window behaves as before); the successor starts its signal thread. Update from the previous release needs nothing else. Not breaking.

Implemented by: [`060c7e4`](https://github.com/tilr-dev/tilrd/commit/060c7e4) fix(reload): let no thread take a held signal while the supervisor execs (2026-10-18).
