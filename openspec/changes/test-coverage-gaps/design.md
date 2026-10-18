# Design

The decision log of the flakes part (tasks 5.x, TILR-76): per flake what was found, what
was done, what was rejected and why. Test code only unless a section says otherwise; no
product behavior changes, so nothing crosses a reload and no client data is involved.

## Context

The harness (`crates/tilrd/tests/common/mod.rs`) starts the real `tilrd` and reads its
stderr on a thread into `Server::log`; `Server::start` returns at `listening on`, which the
server logs before it has finished starting (its host socket, the pane tools, the event
loop come after, `crates/tilrd/src/supervisor.rs` `run`). `local.rs` has its own `Daemon`
for a server started the way `tilr` starts one.

How the flakes were reproduced on macOS: the test alone in a loop next to two to four
loops of the other prebuilt test executables (`target/debug/deps/*`, run directly), not
next to another `cargo` command. Every `cargo test`/`cargo build` re-links
`target/debug/tilrd` (a new inode even when nothing was rebuilt, and another build with
`--workspace` than with `-p tilrd`), and every running test server watches that file and
reloads: a load made with `cargo` beside the tests breaks them in ways CI never sees
(`auth::a_connection_that_never_says_hello_is_closed_after_10_s` failed 11 of 12 that way,
`executable changed on disk` in its log). Linux: the same inside `scripts/linux.sh`.

## D1. ETXTBSY starting a copied tilrd (5.4, 5.14; TILR-110, TILR-125)

Cause: `Daemon::spawn_env` (`tests/local.rs`) spawned with `Command::spawn`. Tests that
copy `tilrd` and start the copy hit `Text file busy` on Linux while a child forked by
another test thread still holds the copy's write descriptor (closed on exec, not on fork).
`common::spawn_copied` already retries that for `Server`; `Daemon` now spawns through it.
- Rejected: copying the executable through a descriptor opened with `O_CLOEXEC` only. The
  descriptor is inherited by any fork in the window between open and the fork's exec
  whatever its flags; retrying the spawn is what passes once that child execs.

## D2. A log line asserted before the reader thread has it (5.5; TILR-115)

Cause: `Server::logged` (and `Daemon::log_text`) read a snapshot of what the reader
thread has collected so far; a line the server writes on an event reaches it some time
after the event, so `assert!(server.logged(..))` right after the event fails under load.
Fix: `Server::wait_logged(needle)` (up to `TIMEOUT`, 20 s) and `wait_logged_within` in
`tests/common`, and every positive `assert!(server.logged(..))` in `crates/tilrd/tests`
became a wait (agent, auth, cloud, exec, panes, reconnect, reload, socks, update; the
`eventually`/`until` polls of a log line and reload.rs's own `wait_logged` went to the shared
one; the 200 ms sleeps "the log thread may still be reading" are gone). In `local.rs` the
`Daemon`'s `wait_logged` waits up to `TIMEOUT` with a deadline and the log asserts there
wait too; `Daemon::exit` waits for the reader to reach the end of the log (end of file)
instead of sleeping 50 ms.
- Negative asserts (`!server.logged(..)`) stay snapshots: absence cannot be waited for;
  they can pass too early, not fail spuriously.
- Rejected: reading the log synchronously in `logged` (no thread). The server writes all
  the time; something has to drain the pipe or the server blocks on its stderr.
