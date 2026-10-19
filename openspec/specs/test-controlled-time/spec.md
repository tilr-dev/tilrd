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
