# Design

What is known about the flake (task 1.1, TILR-111). Moved from the design of the former
shared change `test-coverage-gaps` ("Not fixed here").

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

## Not reproduced yet (1.1; TILR-111)

Not reproduced: 60 runs on macOS beside two loops of the other test executables, 40 runs
in `scripts/linux.sh` beside three such loops, and 30 runs of the whole reconnect
executable there (its tests run in parallel, as in the failing full run). Read in the
code: a `Subscribe` adds the client to the pane's subscribers and asks the pane process
for a full frame; a diff that reaches the new client before that full frame is dropped by
the test's `Screen` (no rows yet) and is in the full frame anyway; frames skipped for a
lagging client are followed by a full frame (on `ClientDrained` and on every tick). Either
order of the test's `sleep 1` and the reconnect brings the row, in the full frame or in a
diff after it. No cause found and nothing changed; the task stays open. If it recurs, the
panic prints the screen, and the server log at debug shows `client lags, skipping frames`.

