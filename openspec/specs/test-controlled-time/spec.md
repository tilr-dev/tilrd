# test-controlled-time Specification

## Purpose
Allow integration tests to provide one shared clock for pane timestamps, frame timing, and raw SOCKS negotiation while preserving the production clock behavior.

## Requirements

### Requirement: Test-controlled pane-use timestamps
In a `failpoints` build, when `TILR_TEST_TIME_FILE` names a readable file containing an unsigned decimal Unix timestamp in microseconds, the supervisor SHALL use its value for `PaneInfo.created_at_ms` and `used_at_ms`. It SHALL observe atomic replacements of the file on subsequent readings. Without this setting, and in a build without `failpoints`, these fields SHALL use the real Unix clock.

**Verified by**: `panes::used_at_ms_follows_subscribe_and_input`.

#### Scenario: Pane use follows a known clock
- **WHEN** a test creates a pane at one clock value, advances the file to a second value, subscribes, advances it again and sends input
- **THEN** `PaneInfo.created_at_ms` is the first value in milliseconds and `used_at_ms` follows the subscribe and input values exactly

### Requirement: Test-controlled pane timing
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the pane SHALL use the shared file's clock for `FrameTiming` durations. Its measurement clock SHALL advance by at least one microsecond on each reading and SHALL observe later atomic replacements of the file. Without this setting, and in a build without `failpoints`, `FrameTiming` SHALL use the real monotonic clock.

**Verified by**: `panes::used_at_ms_follows_subscribe_and_input`.

#### Scenario: A frame build while the test clock is fixed
- **WHEN** the pane builds a frame with output while the clock file is unchanged
- **THEN** its `FrameTiming.build_us` is positive because the pane's measurement clock advances on each reading

### Requirement: Test-controlled SOCKS negotiation deadline
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the raw SOCKS negotiation deadline SHALL expire after ten seconds of test-clock time when the test signals the session's control port, and a session still negotiating SHALL close. The existing ten-second real-time limit SHALL also remain in force. Without this setting, and in a build without `failpoints`, the deadline SHALL remain ten seconds of real time.

**Verified by**: `socks::a_session_that_does_not_say_where_it_goes_is_closed`.

#### Scenario: A raw SOCKS client stalls
- **WHEN** a client receives its SOCKS method selection, sends no request, and the test advances the shared clock by ten seconds and signals the published control port
- **THEN** the server sends one byte on the control connection to confirm the test-clock deadline expired, then closes the session, and a test can observe both I/O events without waiting ten real seconds
- **AND** closure by the real-time limit does not send that confirmation

### Requirement: Test-controlled SOCKS wake-up port
In a `failpoints` build with `TILR_TEST_TIME_FILE`, each raw SOCKS session SHALL publish a local wake-up port in a sibling file named for the client's TCP port before sending its SOCKS method selection.

**Verified by**: `socks::a_session_that_does_not_say_where_it_goes_is_closed`.

#### Scenario: The method selection proves the port is ready
- **WHEN** a raw SOCKS client receives its method selection
- **THEN** the port file exists and the test can connect to that port to make the server recheck its test clock

### Requirement: Test-controlled frame stage durations
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the pane SHALL support `TILR_TEST_FRAME_STAGE_TIME_FILE` assigning exact parse, build and encode durations by advancing only its measurement clock. It SHALL preserve parse accumulation and previous-frame encode reporting. Without this setting, existing timing SHALL remain unchanged. The hook SHALL be compiled out without `failpoints`.

**Verified by**: `metrics::frame_stage_durations_are_controlled_per_operation`.

#### Scenario: Distinct operation durations
- **GIVEN** the stage file contains three whitespace-separated positive decimal microsecond durations in parse, build, encode order
- **WHEN** a test fixes the shared timestamp and sets the stage durations to distinct values
- **THEN** each emitted frame reports parse equal to the configured parse duration multiplied by its output read count and build equal to the configured build duration
- **AND** its encode value equals the duration configured for the previous emitted frame, initially zero
- **AND** scheduling and operational deadlines retain their existing clocks

#### Scenario: Configuration changes between frames
- **WHEN** the test atomically replaces the stage file after receiving a frame and requests another frame from a pane with no pending output
- **THEN** the next frame reports the new build duration and the old encode duration
- **AND** the following frame reports the new encode duration

#### Scenario: Invisible output accumulates across separate reads
- **GIVEN** a `failpoints` build with `frame-stage-no-frame=pause`
- **WHEN** a frame build emits no frame after reading invisible output
- **THEN** the pane hits `frame-stage-no-frame` before returning from the build
- **AND** pausing there lets the test release invisible output across separate reads and verify accumulated parse duration on the next visible frame

### Requirement: Test-controlled successor check deadline
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the successor check deadline SHALL expire after ten seconds of test-clock time when the test signals its published local control port. The server SHALL acknowledge test-clock expiration with one byte. The existing ten-second real-time upper bound SHALL remain in force. Without this setting, and in a build without `failpoints`, the deadline SHALL retain its real-time behavior.

**Verified by**: `reload::a_successor_that_never_answers_the_check_is_not_reloaded_into`.

#### Scenario: A successor never finishes its check
- **WHEN** a successor has started its check and its control port is ready, and a test advances the shared clock by ten seconds and signals the port
- **THEN** the server acknowledges test-clock expiration, kills and reaps the successor, and reports the rejected check without waiting ten real seconds
- **AND** closure through the real-time upper bound does not send that confirmation

### Requirement: Test-controlled Hello deadline
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the Hello deadline SHALL expire after ten seconds of test-clock time when the test signals its published local control port. The server SHALL acknowledge test-clock expiration with one byte. The existing ten-second real-time upper bound SHALL remain in force. Without this setting, and in a build without `failpoints`, the deadline SHALL retain its real-time behavior.

**Verified by**: `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello`.

#### Scenario: A WebSocket client never sends Hello
- **WHEN** a client completes its WebSocket handshake and observes the control-port readiness event, sends no Hello, and the test advances the shared clock by ten seconds and signals that port
- **THEN** the server acknowledges test-clock expiration, closes the connection with 4002 `timeout`, and increments `no_hello` without waiting ten real seconds
- **AND** closure through the real-time upper bound does not send that confirmation

#### Scenario: A client has already sent Hello
- **WHEN** a greeted client's test-clock deadline is signaled after ten test seconds
- **THEN** the server acknowledges the signal and keeps that client connected
