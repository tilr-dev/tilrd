# Design

## Context

At c888b6d the supervisor holds SIGUSR1, SIGHUP, SIGTERM and SIGINT across a reload's exec by blocking them on the exec'ing thread in `pre_exec` (`crates/tilrd/src/reload.rs:285`); the successor finds them blocked, records what is pending (`take_held_signals`, `reload.rs:336`) and acts on the record once its handlers are in (`supervisor.rs:508`). The handlers themselves are tokio's (signal-hook-registry), installed process-wide; any thread without the signal blocked may run them. The standard library's `Command` no longer resets the signal mask in a child (`posix_spawn` without `POSIX_SPAWN_SETSIGMASK`, and `do_exec` does not call `pthread_sigmask`): a child inherits the mask of the thread that starts it. (The archived design of `startup-signal-window`, D1, says the opposite; it was true of older Rust.)

## Decisions

### D1. One thread takes the signals; it blocks them before an exec
The server command blocks the four on the main thread after `record_signals()` and before the runtime is built (`main.rs`), so every thread started afterwards, tokio's or a library's, inherits them blocked. A `signals` thread started then unblocks them for itself and waits on a condition variable; it is the only thread the handlers run on. `hold_everywhere()` asks it to block them and waits until it has; the supervisor calls it right before `command.exec()`; `release_everywhere()` undoes it after a failed exec. This is how a multi-threaded program makes signal delivery predictable (one thread takes them; the rest block them), as daemons that `sigwait` in a dedicated thread do.
- Rejected: blocking on every thread only at exec time. The mask is per thread, and there is no way to reach every thread (tokio's blocking pool, `notify`'s FSEvents and inotify threads).
- Rejected: `SIG_IGN` across the exec (an ignored disposition survives an exec). A signal sent while ignored is discarded, not held: a SIGTERM in the window would be lost.
- Rejected: replacing tokio's signal streams with `sigwait` on the signal thread. Same delivery guarantee, more code; the handlers can stay.

### D2. Children start with nothing blocked
A child inherits the spawning thread's mask, so `spawn_unblocked(|| command.spawn())` lets the four in on the spawning thread for the duration of the spawn and blocks them again; `posix_spawn` stays in use (a `pre_exec` that unblocks would force fork + exec in a multi-threaded process). The pane process, the successor checks (`state-formats`, `CHECK_ENV`), `exec` programs and `cloudflared` go through it. While a thread is in it, that thread can take a signal; so `hold_everywhere()` waits until no thread is spawning and no spawn starts until the exec is over (or failed).

### D3. Failpoint and test
`reload-exec`, after `hold_everywhere()` and right before `command.exec()`: a `sleep:MS` there stops the supervisor in the window. `reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor` sends SIGTERM during it and expects `SIGTERM came during the reload`, exit 0 and the shell gone. Before the change a worker took the SIGTERM and the old image dropped it: the server kept running. The in-kernel race (macOS) itself cannot be hit deterministically; a flood of SIGUSR1 at 0.2 ms killed the server in 3 of 5 ten-second runs before the change and in none of 13 after.

## Reload and reconnect

Nothing new crosses the exec: the held signals are the mask and the pending set, as before. A successor from this change starts its own signal thread; a predecessor from before it holds them on its exec'ing thread only, so a reload into this version can still lose a signal in that one exec. Clients are not involved.

## Backpressure

Not applicable: no client data.
