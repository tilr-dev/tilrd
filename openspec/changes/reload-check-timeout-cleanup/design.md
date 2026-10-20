# Investigation: TILR-170

## Confirmed failure

The original log `/tmp/TILR-85-workspace.log:430-450` records a state-formats
timeout, then a missing-log assertion in `wait_logged_within`, a poisoned mutex
panic in the reader thread, and another in `Server::drop`. Formatting the
assertion's message locks `Server::log`; the temporary guard lives through the
panic. Cleanup then unwraps the poisoned lock while already unwinding and aborts.
The blocking log wait has the same guard lifetime.

The cleanup regression starts a real test server in a subprocess and forces the
missing-log assertion immediately with a zero deadline. It checks ordinary test
failure, preserved diagnostics and removal of the server's scratch directory.
A third case deliberately poisons the log before unwinding to cover cleanup
when a different user of the shared log panics.

The regression failed before the fix in all three cases with SIGABRT and
`PoisonError` (`../logs/harness-before-fix.log`). After copying diagnostics in a
helper that releases its guard before returning, and recovering the log during
cleanup and reads, all three subprocesses exit with ordinary test failure (101)
and remove their scratch directories (`../logs/harness-after-fix.log`).

## State-formats: cause not established

The original log proves the 10-second probe deadline expired, but does not say
whether the child remained alive, exited without being observed, or left stdout
open. `wait_with_output` waits for both the exit and stdout EOF.

Source inspection found no conflicting child reaper: the pty wait targets its
own pid, and the exec observer uses `waitid(P_PID, ..., WNOWAIT)` for its own pid.
The held-signal set excludes SIGCHLD. The state-formats command does not start a
Tokio runtime or the server's signal thread. These observations do not establish
the timeout's cause.

One baseline run reached the command-line refusal successfully:
`cargo test -p tilrd --test reload a_successor_that_rejects_the_command_line_is_not_reloaded_into --features tilrd/failpoints --locked`.
Log: `../logs/rejects-baseline.log`. This is not a reproduction or proof of a fix.
No retries, increased deadlines or speculative probe fixes are justified.
