# Design

## Context

At 44d615a a reload drains the supervisor's events a last time (`supervisor.rs:1881`), detaches the pane writers and holds the tunnel (`supervisor.rs:1896-1921`), then blocks the held signals on every thread (`hold_everywhere`, `reload.rs:420`) and calls `Command::exec` (`supervisor.rs:2055`); a pane process execs with `Command::exec` too (`pane.rs:639-641`). The standard library's `do_exec` sets SIGPIPE to `SIG_DFL` before `execvp` and leaves it so when `execvp` fails. Children are started through `spawn_unblocked` (`reload.rs:316-336`), which `hold_everywhere` waits for.

## Decisions

### D1. SIGPIPE is set back to ignored right after a failed exec
Right after `command.exec()` returns, in the supervisor and in the pane process, `signal(SIGPIPE, SIG_IGN)`, before anything is logged or written. The Rust runtime ignores SIGPIPE at start for the same reason: an error from `write` is handled where it happens.
- Rejected: `-Zon-broken-pipe` (keeps `do_exec` from touching SIGPIPE): unstable, nightly only.
- Rejected: `MSG_NOSIGNAL` / `SO_NOSIGPIPE` on every descriptor: it does not cover pipes (stderr) and has to be repeated at every socket.
- Rejected: exec through `libc::execv` directly: loses what `Command` does for the environment and argument conversion, and the successor would then start with SIGPIPE ignored, which `Command` resets for good reason (children of the successor).

### D2. A stop between the last drain and the exec: an armed flag, raised again after the hold
At the last drain a recording handler (the `record_held` chain, with `HELD_TAKEN` off for the window) is armed for SIGTERM and SIGINT: it sets an async-signal-safe flag. After `hold_everywhere`, if a flag is set, the supervisor sends the signal to itself (`kill(getpid(), sig)`): blocked on every thread, it stays pending across the exec, and the successor records it (`take_held_signals`) and shuts down, as for a signal during the exec. If the exec fails, the flag becomes an `Event::Shutdown`.
- Rejected: draining `events` again after `hold_everywhere`. The path from the handler to the queue (signal-hook pipe, tokio's signal driver, the `shutdown_signal` task, the channel) is asynchronous: a signal taken just before the hold may not have reached the queue yet, so a second drain narrows the gap but does not close it.
- Rejected: holding the signals from the last drain on. The detach and the tunnel wait take up to several seconds, and a stop asked for then would wait for the successor; it also keeps the event loop from seeing SIGUSR1 while it still runs.

### D3. `spawn_unblocked` restores its state in a drop guard
An RAII guard does what follows `spawn()`: first it re-blocks the held signals on the thread, then decrements `spawning` and notifies, so the count never says "no spawn" while the thread can still take a signal. Its lock is taken with `unwrap_or_else(PoisonError::into_inner)`, so the guard never panics in `Drop` (a panic during unwinding aborts). A panic in `spawn()` then leaves nothing behind. Possible sources: `tokio::process::Command::spawn` outside a runtime ("there is no reactor running"), a poisoned mutex, an `unwrap` / `expect` in the window.
- Belt and braces: `hold_everywhere` waits with `Condvar::wait_timeout` (about 5 s); on timeout it logs an error and the reload is cancelled (the signal thread is let go again), instead of freezing the event loop on the main thread.
- Rejected: `catch_unwind` around `spawn()`: needs `UnwindSafe` for every closure and still has to restore the same state; the guard covers early returns too.
- Rejected: a timeout only: it turns a hang into a refused reload every time after one panic, since the count stays wrong.

### D4. The SIGTERM test waits for the failpoint, not for a time
The supervisor logs a line right before the `reload-exec` failpoint, after `hold_everywhere`; `reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor` sends SIGTERM after that line. A second test puts a `sleep:1500` failpoint after the last drain and sends SIGTERM there (gap D2).
- Rejected: a longer fixed delay: still a guess under load, and it makes the test slower.

### D5. The main thread
The exec runs on the main thread, in the future `block_on` drives (`supervisor.rs:373-376`). Whether the hold needs that on macOS is checked (an exec from a tokio worker under a SIGUSR1 flood) and the answer written at the exec site; nothing changes if it does not.

### Dropped: the pane process's exec
The pane process installs no signal handlers, so a signal during its exec has the default action as at any other time; holding signals there would change nothing observable. Only D1 applies to it.

## Reload and reconnect

Nothing new crosses the exec but a signal raised pending (D2), which the successor already reads. A failed exec carries on as before, now with SIGPIPE ignored. Clients are not involved.

## Backpressure

Not applicable: no client data.

## Decision log (implementation)

- **D1, test.** `execvp` is made to fail for real, after the standard library's preparation, with a failpoint `reload-exec-missing` that has the exec run a path with nothing there (`<exe>.missing`); `reload::a_broken_pipe_after_a_failed_exec_does_not_kill_the_server` reads the server's stderr itself and closes it after `reload failed`, then connects (a log line to the dead pipe). Red without the fix (the server died of SIGPIPE). The pane process gets the same one-line restore; it has no test of its own (by review).
- **D2, where it is armed: at the start of `reload()`, not at the last drain.** A signal whose handler ran just before an arm at the drain could still be on its way to the queue (D2's own rejected option: the path to the queue is asynchronous) and be missed by both. Armed when the reload starts, a stop is either noted (its handler ran after the arm) or reached the queue long before the last drain (the check of the successor runs a child process first). The flag is read when the reload ends without an exec: if it is set, `reload()` returns "shut down" (log `asked to stop during the reload`), whether the exec failed or the reload was refused; the stop has also come as an `Event::Shutdown` then, since the recorder is chained behind tokio's handler. The re-raised signal after a failed exec is taken by tokio's handler again, which nobody waits on any more: harmless.
- **D3, test: cloudflared's start, not a pane spawn.** A pane process is started on the main thread, inside the future `block_on` drives: a panic there ends the server, guard or not. The one start of a child the server survives a panic in is `cloudflared`'s, on a tokio task; `cloud::a_panic_starting_cloudflared_does_not_hold_up_the_next_reload` panics there (`spawn-unblocked=panic`, after the count is raised and the signals are let in) and then reloads. Red without the guard (the reload failed after the 5 s hold timeout). The timeout path itself has no test of its own (by review): only a lost decrement could reach it, which the guard rules out.
- **D4, the log line comes from the failpoint.** A `sleep:` failpoint logs `failpoint NAME: sleeping MS ms` before it sleeps, instead of a line of its own in the supervisor: a release has no extra log line, and every sleeping failpoint can be waited for. The new test waits for it at `reload-after-drain`, the changed one at `reload-exec`.
- **D5, answer: the hold does not need the main thread.** Checked on macOS (Darwin 25): with the exec moved to a tokio worker (temporary patch), 80 reloads under SIGUSR1 every 20 ms and five runs of the storm, SIGTERM and held-signal tests all survived. What ended the process was another thread taking a signal during the exec, which the hold rules out whichever thread execs. Written at the exec site; nothing else changes.
