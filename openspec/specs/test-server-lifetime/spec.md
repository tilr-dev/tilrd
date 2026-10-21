# test-server-lifetime Specification

## Purpose
Define integration-test ownership of real servers so panic, interruption and
forced timeout cannot leave supervisors, panes or shells running after the test.

## Requirements

### Requirement: Test-owned server lifetime
Integration test harnesses SHALL establish server ownership before readiness.
An owned server SHALL shut down when its ownership connection closes, including
when a panic, termination signal or forced timeout prevents test destructors.
Cleanup SHALL use owned process exit events, independently of the command socket
and executable pathname, before removing test files.

**Verified by**: `cleanup_stand::owned_servers_exit_after_panic_signal_and_timeout` and
`client::a_panicking_test_stops_its_copied_daemon_without_a_command_socket`.

#### Scenario: Panic after readiness
- **WHEN** a test panics after starting a server
- **THEN** its server exits and cleanup observes its process exit

#### Scenario: Panic before readiness
- **WHEN** startup unwinds before constructing a ready server
- **THEN** ownership still requests server shutdown

#### Scenario: Test receives a termination signal
- **WHEN** the test process receives SIGTERM after server readiness
- **THEN** the server observes ownership loss and exits

#### Scenario: Test is killed after a controlled timeout
- **WHEN** a blocked ready test exceeds its controlled wait and is killed
- **THEN** its server observes ownership loss and exits

#### Scenario: Copied daemon loses its command socket
- **WHEN** a client test starts a private copied daemon and its command socket is removed
- **THEN** ownership cleanup still stops that daemon

#### Scenario: Server reloads
- **WHEN** an owned server replaces its executable in place
- **THEN** the replacement retains ownership and exits when the test disappears
