## ADDED Requirements

### Requirement: Test-controlled successor check deadline
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the successor check SHALL expire after ten seconds of test-clock time when the test signals its published local control port. The server SHALL acknowledge test-clock expiration with one byte on the control connection before rejecting the successor. The existing ten-second real-time upper bound SHALL remain in force, and expiration through that real-time bound SHALL NOT send the acknowledgement. Without this setting, and in a build without `failpoints`, the check SHALL retain its ten-second real-time deadline.

**Verified by**: `reload::a_successor_that_never_answers_the_check_is_not_reloaded_into`.

#### Scenario: A successor never finishes its check
- **WHEN** a successor has started its check and its control port is ready, and a test advances the shared clock by ten seconds and signals the port
- **THEN** the server acknowledges test-clock expiration, kills and reaps the successor, and reports the rejected check without waiting ten real seconds

### Requirement: Test-controlled Hello deadline
In a `failpoints` build with `TILR_TEST_TIME_FILE`, the Hello deadline SHALL expire after ten seconds of test-clock time when the test signals the connection's published local control port. The server SHALL acknowledge test-clock expiration with one byte on the control connection before closing the WebSocket with 4002 `timeout` and counting `no_hello`. The existing ten-second real-time upper bound SHALL remain in force, and expiration through that real-time bound SHALL NOT send the acknowledgement. Without this setting, and in a build without `failpoints`, the Hello deadline SHALL retain its ten-second real-time behavior.

**Verified by**: `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello`.

#### Scenario: A WebSocket client never sends Hello
- **WHEN** a client completes its WebSocket handshake and observes the control-port readiness event, sends no Hello, and the test advances the shared clock by ten seconds and signals that port
- **THEN** the server acknowledges test-clock expiration, closes the connection with 4002 `timeout`, and increments `no_hello` without waiting ten real seconds
