# test-client-lifetime Specification

## Purpose
On 2026-10-31, the owner observed orphaned fake-server clients left running
after successful integration test runs. The terminal harness neither reaped
clients on drop nor prevented them from inheriting PTY master descriptors.

## Requirements

### Requirement: Test-owned client lifetime
Integration-test terminal clients SHALL exit and be reaped when their owning
terminal is dropped, including during panic unwind. Executed child processes
SHALL NOT inherit PTY descriptors except their assigned standard streams.

**Verified by**: `client::terminal_drop_reaps_its_client_even_during_panic` and
`client::terminal_children_do_not_inherit_pty_descriptors`.

#### Scenario: Terminal leaves scope
- **WHEN** a test drops a terminal with a live attached client
- **THEN** that client is terminated and reaped before cleanup returns

#### Scenario: Test panics
- **WHEN** a test unwinds with a live attached client
- **THEN** that client is terminated and reaped during terminal cleanup

#### Scenario: Another terminal starts
- **WHEN** a child executes while another test terminal exists
- **THEN** it receives neither its own PTY master nor the other terminal's PTY descriptors
