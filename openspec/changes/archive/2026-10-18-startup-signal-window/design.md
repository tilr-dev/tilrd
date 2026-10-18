# Design

## Context

At 048b0e2 the supervisor's sockets are served (`crates/tilrd/src/supervisor.rs:384-405`, accept loops spawned at `supervisor.rs:471-490`) before its signal handlers exist (`supervisor.rs:495-500`; SIGINT lazily through `ctrl_c()` in `shutdown_signal`, `supervisor.rs:796-803`). The reload already has a mechanism for "signals before the handlers": the exec'd image starts with SIGUSR1, SIGHUP and SIGTERM blocked (`reload::hold_signals`, `crates/tilrd/src/reload.rs:285-297`); `take_held_signals` (`reload.rs:335-357`), called first thing in `main` (`crates/tilrd/src/main.rs:250`) while the process has one thread, installs a handler that only records them (`record_held`, `reload.rs:318-326`) and unblocks them; `take_held_record` (`reload.rs:360-370`) stops the recording and the supervisor acts on the record (`supervisor.rs:501-512`). tokio's handlers (signal-hook-registry) chain to the handler that was installed before them, which is why `record_held` checks `HELD_TAKEN`.

## Decisions

### D1. Record, not block, on a fresh start
For the server command (`Cli.command == None`, `main.rs:279-283`), after `Cli::parse()` and before `supervisor::run` builds the tokio runtime, install `record_held` for SIGTERM, SIGINT, SIGHUP and SIGUSR1 with `sigaction` whatever the mask (the reload path keeps its "only what arrived blocked" check, then unblocks). A disposition is process-wide, so it covers the tokio worker threads started later without touching their masks; the supervisor's existing `take_held_record()` after its handlers turns the record into events.
- Rejected: blocking the four signals with `pthread_sigmask` before the runtime starts and unblocking after the handlers. The mask is per thread and every tokio worker (and blocking-pool thread) inherits it at creation; unblocking afterwards would have to run on each of them, and a thread created later from a blocked one would stay blocked. The recording handler needs none of that. (Children are not affected either way: std's `Command` resets the mask in the child, which is why `hold_signals` sets it in `pre_exec`.)
- Rejected: installing the handlers before binding. `tokio::signal` needs the runtime and the events channel, and the handlers' consumers need the supervisor; moving them up spreads the start-up order over more code than recording does.

### D2. The set is one list: SIGTERM, SIGINT, SIGHUP, SIGUSR1
`HELD` (`reload.rs:266`) gains SIGINT and `HeldSignals` gains `interrupt`; the reload's held set and the startup set are the same list. SIGINT is acted on as SIGTERM is (`Event::Shutdown`). On a reload this closes the same window for a SIGINT (a foreground `tilrd` getting Ctrl-C while it reloads).

### D3. SIGINT's handler with the others
`shutdown_signal` takes a `signal(SignalKind::interrupt())` stream created next to `term` (`supervisor.rs:495`) instead of calling `ctrl_c()` inside the spawned task: `take_held_record()` stops the recording at once, and a SIGINT between it and the task's first poll would otherwise be recorded by nobody and handled by nobody (dropped, not fatal, but lost).

### D4. Log lines
A held SIGTERM or SIGINT logs `SIGTERM came while the server started` / `SIGINT came while the server started` on a fresh start and keeps `SIGTERM came during the reload` (and the SIGINT counterpart) after a reload, then the usual `shutting down, closing N pane(s)`. A held SIGUSR1 on a fresh start logs `SIGUSR1 came while the server started: reloading` and reloads once (into the same executable if it did not change): consistent with what SIGUSR1 does, and harmless.

### D5. Failpoint for the test
`crate::fail::point("supervisor-before-signals")` right before the handlers are installed (`supervisor.rs:494`), after the accept loops are spawned, so a test can see the socket accept and then signal into the window. `fail::point`'s `sleep:MS` blocks the thread running `block_on`, which is the point: the accept loops run on the workers.

## Reload and reconnect

A reload execs with the four signals held (`hold_signals`) and the successor records them as today, SIGINT included. Nothing new is written to the state files; `state-formats` is unchanged. A predecessor from before this change holds only three; a SIGINT during that exec still takes the default action once (no worse than now). Clients are not involved: a shutdown in the window closes their connections like any shutdown.

## Backpressure

Not applicable: no client data.
