# Proposal: a signal while the server starts stops it cleanly

## Why

Itsaplan TILR-106 (phase 1, bug). Seen as a flake of `local::a_server_that_stops_removes_its_socket_and_lock` under full-suite load: the test waits until the socket accepts a connection (`crates/tilrd/tests/local.rs:62-83`), sends SIGTERM and expects exit 0 with the socket and lock gone (`local.rs:538-546`). Checked at 048b0e2:

- `supervisor::run` binds the TCP port (`crates/tilrd/src/supervisor.rs:391`, `bind_listener`), takes over or claims the Unix socket (`supervisor.rs:398`, `local::inherit`; `supervisor.rs:401`, `set_up_local`), reads the token, starts the tunnel task and spawns the accept loops (`supervisor.rs:471-490`), and only then installs the SIGTERM, SIGUSR1 and SIGHUP handlers (`supervisor.rs:495-500`). SIGINT's handler comes later still: `tokio::signal::ctrl_c()` is first called when the spawned `shutdown_signal` task is first polled (`supervisor.rs:796-803`).
- On a fresh start nothing covers that window: `reload::take_held_signals` (called at `crates/tilrd/src/main.rs:250`) installs its recording handler only for signals that arrived blocked, i.e. after a reload's exec (`crates/tilrd/src/reload.rs:335-357`, early return at `reload.rs:345-347`). A SIGTERM or SIGINT (Ctrl-C on a foreground `tilrd`) between the bind and `supervisor.rs:495` takes the default action: the process dies with the socket file and `tilrd.sock.lock` left behind and no `shutting down` line; a SIGUSR1 kills it too, a SIGHUP as well.
- The reload path already solves the same problem for the successor (commit a0f1ae9, archived change `2026-10-18-reload-signals-and-socket`): `hold_signals` blocks SIGUSR1, SIGHUP and SIGTERM in the exec'd image (`reload.rs:266`, `reload.rs:285-297`), `take_held_signals` turns them into records, and the supervisor acts on `take_held_record()` once its handlers are in place (`supervisor.rs:501-512`). SIGINT is not in that set (`reload.rs:266`), so a SIGINT during a reload's exec also kills the server.

## What Changes

- A server (`tilrd` with no subcommand) records SIGTERM, SIGINT, SIGHUP and SIGUSR1 from before it binds anything until its own handlers are in place, and then acts on what came: SIGTERM or SIGINT a clean shutdown (socket and lock removed, exit 0), SIGHUP ignored, SIGUSR1 one reload. This reuses the reload's record (`take_held_signals` / `take_held_record`), installed for the server command after the command line is parsed and before the tokio runtime is built.
- SIGINT joins the signals held across a reload's exec, handled by the successor as SIGTERM is.
- SIGINT's handler is installed with the others (not lazily by `ctrl_c()` in a spawned task), so no SIGINT falls between the record and the handler.
- A failpoint between the sockets being served and the handlers being installed (`supervisor-before-signals`), for the tests.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md` (new requirement: signals while the server starts).
- `reload`: see `specs/reload/spec.md` (SIGINT held across the exec).

## Non-goals

- Signals before the command line is parsed: nothing is bound or locked by `tilrd` yet (a `--lock-fd` lock file is `tilr`'s, and a stale lock file is taken over by the next start).
- Pane processes (`tilrd pane`) and the other subcommands keep the default dispositions.
- SIGKILL (cannot be caught; UC-014 covers what the next start does).
- Changing what SIGHUP or SIGUSR1 do once the server runs.

## Impact

`crates/tilrd/src/main.rs` (record the startup set for the server command), `crates/tilrd/src/reload.rs` (the held set gains SIGINT; `HeldSignals` gains `interrupt`; a recording install that does not need the signals to arrive blocked), `crates/tilrd/src/supervisor.rs` (SIGINT stream installed with the others; the record acted on with a log line fitting a fresh start; failpoint), `crates/tilrd/tests/local.rs`, `crates/tilrd/tests/reload.rs`, USE_CASES.md, README "Signals" (`README.md:268-272`). No protocol change; nothing new crosses a reload (the held signals are the process's mask and pending set, as before). An update from the previous release needs nothing: a predecessor without SIGINT in its held set simply does not hold it. Not breaking.

Implemented by: [`f8fb63b`](https://github.com/tilr-dev/tilrd/commit/f8fb63b) fix: handle a signal that arrives while the server starts (2026-10-18).
