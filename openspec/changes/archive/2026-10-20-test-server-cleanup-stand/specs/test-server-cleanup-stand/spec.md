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

#### Scenario: Controlled timeout after ready
- **WHEN** the stand receives the ready event and server PID
- **THEN** it triggers a process wait timeout, terminates the test process, and checks whether that server remains

### Requirement: Owned process accounting and cleanup
The stand SHALL check the exact PID of every server it starts and, on macOS,
check those PIDs against `pgrep -f target/debug/tilrd`. It SHALL terminate its
own process group, including servers whose ready message was never received,
and reap the test process even if a check or the stand itself fails.

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
