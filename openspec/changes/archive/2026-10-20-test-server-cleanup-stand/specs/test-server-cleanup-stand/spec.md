## ADDED Requirements

### Requirement: Deterministic interrupted-test stand
The test-server cleanup stand SHALL start a test-owned tilrd and wait for its
confirmed ready event before interrupting the test process. It SHALL provide
separate signal and timeout cases without sleeps or repeated runs for ordering.

**Verified by**: `scripts/test_server_cleanup_stand_selftest.py` and a real
`scripts/test_server_cleanup_stand.py` run.

#### Scenario: Signal after ready
- **WHEN** the stand receives the ready event and server PID
- **THEN** it signals the test process and checks whether that server remains

#### Scenario: Ready message is split across pipe reads
- **WHEN** the fixture's ready line arrives in fragments, including a split within its PID
- **THEN** the stand waits for the complete newline-terminated message and checks the full PID

#### Scenario: Controlled timeout after ready
- **WHEN** the stand receives the ready event and server PID
- **THEN** it triggers a process wait timeout, terminates the test process, and checks whether that server remains

### Requirement: Owned process accounting and cleanup
The stand SHALL check each owned server PID and, on macOS, compare it with
`pgrep -f target/debug/tilrd`. It SHALL terminate its process group and reap
the fixture on failure, including before readiness. Success SHALL require an
OS exit event; unavailable watchers SHALL fail explicitly. After process
cleanup it SHALL remove its fixture directory and sockets.

**Verified by**: `scripts/test_server_cleanup_stand_selftest.py` and a real
`scripts/test_server_cleanup_stand.py` run.

#### Scenario: A test server survives interruption
- **WHEN** a stand-owned server remains after the interrupted test exits
- **THEN** the stand reports failure and removes that server before exiting

#### Scenario: A test server exits with its test process
- **WHEN** the stand-owned server exits after its test process is interrupted
- **THEN** the signal and timeout checks both pass

#### Scenario: A check fails
- **WHEN** the stand cannot complete a process check
- **THEN** it still removes its test process and server

#### Scenario: Readiness fails
- **WHEN** a test-owned server starts but no valid ready message arrives
- **THEN** the stand terminates that server through its owned process group

#### Scenario: Server shutdown completes after test exit
- **WHEN** the interrupted test has exited and its server is still shutting down
- **THEN** the stand waits for that exact server's exit event and passes when it arrives
- **AND** a 30-second failure deadline bounds reporting a survivor, without sleeps or PID polling

#### Scenario: Exit watcher unavailable
- **WHEN** the OS cannot register a watcher for the ready server PID
- **THEN** the stand reports an explicit error and cleans its owned process group

#### Scenario: The stand receives SIGTERM
- **WHEN** SIGTERM interrupts the stand while it is checking a ready server
- **THEN** it removes its owned process group, reaps the fixture, and exits with status 143

#### Scenario: Interrupted fixture leaves filesystem artifacts
- **WHEN** interruption prevents the fixture from running its destructors
- **THEN** the stand removes its owned scratch directory, notification socket and runtime sockets after terminating its processes
