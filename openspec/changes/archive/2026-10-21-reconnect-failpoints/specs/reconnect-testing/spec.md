## ADDED Requirements

### Requirement: Tests control reconnect stages explicitly

With the `failpoints` feature, tests SHALL be able to pause output produced with
no viewer, client acceptance, subscription, full-frame delivery and the decision
to skip frames for a lagging client. Each hit SHALL use the existing pause
controller handshake and wait for explicit release. Without the feature, the
hooks SHALL have no effect.

**Verified by**: `reconnect::a_test_controls_output_accept_subscribe_and_full_frame_on_reconnect`, `reconnect::a_test_pauses_socket_reads_until_reconnected_client_lags`.

#### Scenario: Output precedes reconnect
- **WHEN** a test holds output produced with no viewer and then releases each reconnect stage
- **THEN** it observes connection acceptance, subscription and full-frame delivery in its chosen order and receives the pane's screen

#### Scenario: Client falls behind
- **WHEN** a test pauses socket reads and produces frames until the client falls behind
- **THEN** it observes the frame-skipping decision and explicitly resumes reads

### Requirement: Tests explicitly resume socket reading

The test client SHALL provide an acknowledged pause of socket reads. Once the
pause is acknowledged it SHALL consume no further socket messages until the
test explicitly resumes reading. Sending client messages SHALL remain possible.

**Verified by**: `reconnect::a_test_pauses_socket_reads_until_reconnected_client_lags`.

#### Scenario: Pause handle disappears
- **WHEN** a test drops the read-pause handle without resuming
- **THEN** its next read fails rather than silently resuming

#### Scenario: Paused client keeps its socket open
- **WHEN** the test pauses a connected client's reads
- **THEN** the connection remains open, the client can send messages and reads resume only when requested

