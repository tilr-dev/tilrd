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
