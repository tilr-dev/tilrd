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

## D3. Using a server before it has finished starting (5.13, 5.15; TILR-120, TILR-126)

Cause: `Daemon::start` returned once a connection to the Unix socket succeeded. The
server binds its socket before it writes its access token (`state/token`) and long before
its event loop runs; connections wait in the backlog until then. So
`local::server_commands_run_for_local_clients_only` could read `state/token` before it
existed (NotFound), and a `tilrd ps` right after the start ran its own 5 s timeout down
while the server was still starting under load (`local::a_socket_left_by_a_killed_server_is_replaced`,
`local::a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket`).
Fix: what a service manager waits for. Each `Daemon` gets a `NOTIFY_SOCKET` of its own and
`Daemon::start` waits for `READY=1`, which the server sends once it has started all the
way (`crate::notify::ready()` right before the event loop); a test that brings its own
`NOTIFY_SOCKET` (`a_service_manager_is_told_when_the_server_is_ready`) keeps waiting for
the socket, and `starting_with_a_window` too (it must return inside the start-up window,
before READY). `tilrd ps` asserts go through `Daemon::assert_ps`, which prints the exit
status, stdout and stderr of `ps` and the server's log when it fails.
- Rejected: polling for `state/token`. It fixes the token read only; the `ps` timeouts
  came from the same early return.
- Rejected: waiting for a log line written late in the start (`SSH agent forwarding`). It
  is not meant as a readiness signal and may move; READY=1 is part of the contract
  (`a_service_manager_is_told_when_the_server_is_ready`).
- Rejected: a longer `tilrd ps` timeout. It is the product's; waiting for the server to be
  ready is what a caller of `ps` does.

## D4. The second server binds the host socket first (5.10; TILR-113)

Cause: `start_second` started the second server once the first had logged `listening
on`, but the first binds its host socket (`<server id>.sock`) only after that
(`set_up_host_socket`); under load the second got the plain name and the first the
`-<pid>` one (`files::two_servers_of_one_user_keep_their_host_sockets`,
`reload::a_second_server_keeps_its_host_socket_across_a_reload`). Which of two servers
gets the plain name is not specified by start order: a test race, not a product bug.
Fix: `start_second(first: &Server)` waits for the first's `host events on`, starts the
second and returns once the second has logged its own `host events on`. Its callers
(files, reload, agent) pass the first server.
- The macOS symptom `tilr edit` exiting 127 in the same test was TILR-122, the shared
  `bin/tilr` removed and linked again by a starting server: fixed in the product by
  72b0422 (links renamed over the old ones). Not seen again here (see "Stability").

## D5. The reload check outlasting the test's wait (5.16, 5.3; TILR-127)

Cause: after the executable is replaced, the server sees it within two looks a
`WATCH_INTERVAL` (1 s) apart and gives the new file up to `CHECK_TIMEOUT` (10 s,
`crates/tilrd/src/reload.rs`) to answer `state-formats`; on macOS under the full reload
suite that child (`/bin/sh <exe>`) sat in `_dyld_start` for more than 8 s. The test gave up
after 10 s in all, before the server, still within its own bounds, logged `not reloading`.
Fix: `reload.rs` (tests) names the two bounds (`WATCH_INTERVAL`, `CHECK_TIMEOUT`, with a
comment pointing at src/reload.rs) and waits `JUDGED` = 2 × WATCH_INTERVAL + CHECK_TIMEOUT
+ 5 s, in `a_broken_executable_is_not_reloaded_into` and in
`an_executable_that_reads_another_state_format_is_not_reloaded_into`, which waits for the
same verdict. A longer wait is right here: the thing waited on is a process start the
server itself allows 10 s for.
- Rejected: a test-only shorter `CHECK_TIMEOUT`. Under that load the check would time out
  instead and the test would pass through `did not finish` rather than the exit status of
  the broken file: it would test another path.
- Rejected: importing the constants. They live in the binary crate, which tests cannot
  import; the copies say where they come from.
