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

The stand is a lifecycle diagnostic: it fails when interrupted test
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
the lifecycle fix moves servers to separate groups, it must extend ownership and
cleanup together rather than bypass this check. Build the fixture with
`tilrd/failpoints`, matching the documented suite's server binary.

The pgrep-failure integration case applies only on macOS, where the stand
uses pgrep. Other integration cases exercise both operating systems.

### Interruption of the stand

SIGTERM raises SystemExit(143), unwinding the same owned-group cleanup used by
check failures. The regression fixture announces a server and the wrapper emits
a pipe event upon entering the server exit wait; only then does the test signal
the stand. The test also kills its fixture group in a finally block so a failing
regression cannot leave its deliberately surviving child behind.

The ready parser accepts only a complete newline-terminated message. Pipe reads
may split the decimal PID anywhere, so an incomplete suffix cannot identify a
server. A deterministic regression feeds `pid=12` and `345\n` as separate reads
and requires PID 12345.

### Fixture filesystem ownership

The stand owns a short `/tmp/tilr-stand-*` directory per case and passes it
to the fixture through `TILR_STAND_DIR`. The short runtime path keeps the
host socket inside that directory on macOS. After process cleanup, a Python
context manager removes the directory even when readiness fails or SIGTERM
interrupts the stand. This replaces fixture-only scratch ownership, whose
Rust destructor cannot run after the test process is killed.

## Supervisor return: orphan exit observation
The SIGTERM selftest used kill(pid, 0) after the stand exited. An orphan zombie
can still satisfy that check until init reaps it. Register a parent-owned exit
watcher before SIGTERM and assert its event after communicate; reaping latency
must not determine the verdict. Model delayed reaping by forcing the old PID
probe to return True. READY uses a named readiness deadline; a vanished PID
gets an explicit error. The fixture blocks on stdin without a release protocol.
