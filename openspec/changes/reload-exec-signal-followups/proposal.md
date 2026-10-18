# Proposal: the rest of a reload's signal handling

## Why

Itsaplan TILR-124 (phase 1, bug). Found while implementing `reload-signal-storm` (060c7e4, its Non-goals) and in a review of that commit. Each point was checked in the code at 44d615a (Rust 1.94.0):

1. **SIGPIPE after a failed exec.** `Command::exec` runs the standard library's `do_exec` in the calling process, which sets SIGPIPE to `SIG_DFL` process-wide before `execvp` (`library/std/src/sys/process/unix/unix.rs:359-370`) and does not restore it when `execvp` fails. tilrd starts with SIGPIPE ignored (the Rust runtime does that) and never sets it again (no `SIGPIPE` in `crates/tilrd/src`). Both execs in place go through it: the supervisor's (`crates/tilrd/src/supervisor.rs:2055`) and each pane process's (`crates/tilrd/src/pane.rs:639-641`). After a failed reload exec, which the server survives by design ("A reload that fails before the handover changes nothing"), a write to a pipe or socket whose reader is gone (stderr to a closed pipe, a pane process's link to a supervisor that went away) kills the process by SIGPIPE, with nothing logged.
2. **A shutdown signal before the exec is lost.** `reload()` drains the event queue a last time (`supervisor.rs:1881`, an `Event::Shutdown` there ends the reload), then waits for each pane's writer to detach (`DETACH_TIMEOUT`, 5 s each, and up to another 5 s after a SIGKILL, `supervisor.rs:1896-1912`) and for the tunnel (`self.tunnel.hold().await`, `supervisor.rs:1921`), and only then holds the signals (`hold_everywhere`, `supervisor.rs:2053`) and execs. A SIGTERM or SIGINT in that gap is taken by the signal thread; `shutdown_signal` (`supervisor.rs:822-832`) turns it into an `Event::Shutdown` that is never read before the exec succeeds, and it `select!`s once, so later ones are swallowed too. The successor runs on: the server ignores the stop. The record-only handler that would catch them (`record_held`, `reload.rs:456`) is off once the server's handlers are in (`HELD_TAKEN`, `reload.rs:518`).
3. **The SIGTERM test can land in that gap.** `reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor` (`crates/tilrd/tests/reload.rs:1021`) sends SIGTERM 500 ms after `reloading`, which is logged before the detach and the tunnel wait (`supervisor.rs:1859`), not at the failpoint `reload-exec=sleep:1500`. On a loaded runner the signal can arrive in gap 2 and the test fails with "the server keeps running". Not observed yet.
4. **`spawn_unblocked` has no guard.** `reload.rs:316-336` increments `spawning`, unblocks the held signals on the thread and calls `spawn()`; a panic in `spawn()` leaves `spawning` incremented and the signals unblocked on that thread, so the next reload's `hold_everywhere` waits forever (`set_signal_thread`, `reload.rs:438`) on the main thread: the event loop freezes.
5. **An undocumented assumption.** The exec runs on the main thread (`supervisor::run` drives the event loop with `block_on`, `supervisor.rs:373-376`); the comment at the exec site says nothing about whether the hold depends on that on macOS.

Dropped from TILR-124 after the check: "the pane process's own reload exec (`pane.rs:638`) holds no signals, the hole 060c7e4 closed for the supervisor". It does hold none, but the pane process installs no signal handlers (no `tokio::signal` or `sigaction` in `pane.rs`), so a signal during its exec does what it does at any other time (the default action); there is no handler to run on a thread of a replaced image and no queued event to lose. Only point 1 applies to the pane process.

## What Changes

- After a failed exec in place (supervisor or pane process) SIGPIPE is ignored again before anything else runs.
- From the last drain of the event queue until the exec, a SIGTERM or SIGINT is recorded where the exec cannot lose it: an async-signal-safe flag set by a recording handler armed at the drain (the `record_held` chain), checked after `hold_everywhere`; if set, the supervisor raises the signal on itself (`kill(getpid(), sig)`), which stays pending, blocked, for the successor (and is handled as before when the exec fails).
- `spawn_unblocked` restores the mask and the count in a drop guard; `hold_everywhere` gives up after about 5 s with an error and the reload is cancelled instead of freezing the event loop.
- The exec site says why it runs on the main thread, or that it need not, after checking it on macOS.
- Tests: the SIGTERM test waits for a log line at the `reload-exec` failpoint instead of a fixed 500 ms; a new test sends SIGTERM in the gap before the exec (a failpoint after the drain, e.g. at `self.tunnel.hold()`), and one checks that the server survives a broken pipe after a failed exec.

## Capabilities

### Modified Capabilities
- `reload`: new requirements "A failed exec leaves SIGPIPE ignored" and "A stop asked for while a reload prepares its exec is not lost".

## Non-goals

- Signals to a pane process during its exec (no handlers there; see Why).
- What the four held signals do once the server runs, and the hold during the exec itself (060c7e4).
- Making the exec never fail.

## Impact

`crates/tilrd/src/reload.rs` (recording handler, SIGPIPE restore, `spawn_unblocked` guard), `crates/tilrd/src/supervisor.rs` (`reload()`, `exec_successor`), `crates/tilrd/src/pane.rs` (SIGPIPE restore), `crates/tilrd/tests/reload.rs`, README reload section, USE_CASES.md. No protocol change; nothing new crosses a reload except a signal raised pending, which the successor already reads (`take_held_signals`); `state-formats` unchanged. A reload from the previous release into this one: the predecessor's gap stays (its code); the successor handles a pending signal as before. Update from the previous release needs nothing else. Not breaking.
