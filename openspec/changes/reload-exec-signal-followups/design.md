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
