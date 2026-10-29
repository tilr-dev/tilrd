# reconnect-testing Specification

## Purpose
Provide deterministic control of reconnect stages and test-client socket reads so
integration tests can reproduce reconnect ordering and lag recovery explicitly.

## Requirements

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

**Verified by**: `reconnect::a_test_pauses_socket_reads_until_reconnected_client_lags`, `reconnect::a_dropped_read_pause_fails_the_next_read`.

#### Scenario: Pause handle disappears
- **WHEN** a test drops the read-pause handle without resuming
- **THEN** its next read fails rather than silently resuming

#### Scenario: Paused client keeps its socket open
- **WHEN** the test pauses a connected client's reads
- **THEN** the connection remains open, the client can send messages and reads resume only when requested

### Requirement: Fresh test shells are ready before input

Shared test helpers that open and subscribe to an interactive shell SHALL wait
for its first prompt before returning. Tests that deliberately send input before
startup SHALL retain explicit control of that ordering.

**Verified by**: `reconnect::opening_a_fresh_pane_waits_for_the_first_shell_prompt`, `reconnect::reconnect_output_when_input_precedes_the_shell_prompt`, `reconnect::shell_readiness_ignores_prompts_from_other_panes`.

#### Scenario: Shell startup is held
- **WHEN** a test holds the shell before its first prompt and opens a pane through the shared helper
- **THEN** opening remains pending until startup is released and the prompt is observed
- **AND** the first command's output can be matched as an exact row without adding a leading newline

#### Scenario: A test deliberately precedes startup
- **WHEN** a test explicitly uses the controlled startup fixture
- **THEN** it can observe PTY input echo before releasing the shell's first prompt

#### Scenario: Another subscribed pane prints a prompt
- **WHEN** a client opening a new shell receives a prompt frame from an older pane
- **THEN** only a prompt frame belonging to the newly opened pane can complete opening
