# Design

What was found about the flake (task 1.1, TILR-117). Moved from the design of the former
shared change `test-coverage-gaps` (D8 there); the reproduction method of its Context applies.

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

## The login shell outliving the hang-up (1.1; TILR-117; formerly D8)

`reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up` waited 5 s
for the login shell (`sh -c 'tilrd & ...; wait'`, session leader of a pty) to exit after
the master is closed and its process group gets SIGHUP. Measured: the join of the reader
thread 2-23 ms, the shell gone at the first look (0.1 s) in 20 of 20 runs with four loops of
the other test executables beside it; it failed once with a Linux VM and Dagger saturating
the Mac. No mechanism in the test or the server that would hold the shell was found (no
SIGHUP disposition inherited from the tests, the master's and slave's descriptors are
close-on-exec, the shell is our child and is reaped by `try_wait`). The wait is on the
kernel and the scheduler; it now looks for up to `TIMEOUT` (`eventually_within`, a new
helper next to `eventually`, which keeps its 5 s), with the reason in the test.
- Rejected: changing `eventually`'s 5 s for everyone: the other waits were not seen
  failing.

