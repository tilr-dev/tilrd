## MODIFIED Requirements

### Requirement: Tests explicitly resume socket reading

The test client SHALL provide an acknowledged pause of socket reads. Once the
pause is acknowledged it SHALL consume no further socket messages until the
test explicitly resumes reading. Sending client messages SHALL remain possible.

**Verified by**: `reconnect::a_test_pauses_socket_reads_until_reconnected_client_lags`, `reconnect::a_dropped_read_pause_fails_the_next_read`.

#### Scenario: Pause handle disappears
- **WHEN** a test drops the read-pause handle without resuming
- **THEN** its next read fails rather than silently resuming

#### Scenario: Paused client keeps its socket open
- **WHEN** the test pauses a connected client's reads
- **THEN** the connection remains open, the client can send messages and reads resume only when requested
