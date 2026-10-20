# Design

An integration-test fixture starts one real `target/debug/tilrd` with a
test-owned `NOTIFY_SOCKET`. It waits for the server's `READY=1` datagram,
then writes its PID to a control pipe and blocks reading that pipe. The
external stand waits for that specific message before interrupting the test
executable. `listening on` is too early: the server is not fully started then.

The stand runs the fixture twice in separate processes. In the signal case
it sends SIGTERM to the test process. In the timeout case it starts a short
watchdog only after READY, requires `TimeoutExpired`, and kills that process.
The watchdog tests a deliberately blocked fixture; it does not synchronize
startup. The ready message provides the ordering in both cases. The stand
checks the exact server PID after each interruption and on macOS intersects
the PID with `pgrep -f target/debug/tilrd`, so unrelated servers do not
affect the result. It reports leaks as failures. A `finally` block kills the
stand-owned process group and reaps the test process on every path, including
failure before READY. When the server PID is known, it waits for its exit
event after cleanup. The fixture is ignored by the full test suite.

The stand is a diagnostic for TILR-153: it fails when interrupted test
servers survive and passes once that lifecycle bug is fixed. Synthetic
integration checks use both a surviving child and one that exits when its
test process dies; they prove red and green outcomes independently of that
pending fix. The stand adds no product state, reload handoff, or reconnect
behavior.

## Exit event and bounded failures

Register a kqueue process-exit watcher on macOS or pidfd on Linux after
READY and before interrupting the fixture. A missing or failed watcher is
an explicit error; a `kill(pid, 0)` snapshot cannot substitute for an exit
event. Wait for the event after the fixture exits. A 30-second deadline
(the same failure bound as READY) only bounds reporting a surviving server;
it never delays a successful verdict once the event arrives. Cleanup uses
the same event watcher after killing the owned group.

The ready PID must currently belong to the fixture process group. If
TILR-153 moves servers to separate groups, it must extend ownership and
cleanup together rather than bypass this check. Build the fixture with
`tilrd/failpoints`, matching the documented suite's server binary.

The pgrep-failure integration case applies only on macOS, where the stand
uses pgrep. Other integration cases exercise both operating systems.
Script-based `Verified by` references are not recognized by the coverage
parser, so these two requirements intentionally appear uncovered there.
