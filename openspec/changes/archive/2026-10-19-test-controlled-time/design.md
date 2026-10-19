# Design: test-controlled time

## Decision

The test supplies one unsigned decimal Unix timestamp in microseconds in an atomically replaced file named by `TILR_TEST_TIME_FILE`. In a `failpoints` build, the supervisor reads it for `PaneInfo` milliseconds and the pane maps it to a process-local `Instant` base for timing durations. The pane clock advances by at least one microsecond per measurement even while the file is unchanged, so a frame build still has positive duration. Both server and pane inherit the path. A test changes the file only between protocol events and waits for the resulting protocol response before asserting, so it never relies on wall-clock scheduling. Without the variable, the production clock path is unchanged.

For raw SOCKS, a `failpoints` build with a shared test clock binds a local control listener before negotiating and writes its port to a sibling file named for the client's TCP port. The test-clock deadline is ten seconds; the existing ten-second real-time limit also remains in force. The test receives the method selection (which proves the listener is ready), advances the shared clock by ten seconds through atomic replacement, signals that listener, waits for the session to close, then checks that the server still answers the main protocol. The ordinary build uses Tokio's ten-second real timer. No polling sleep is required.

## Alternatives

- Tokio paused time alone cannot control the separately spawned server and pane processes in integration tests.
- Test-only sleeps still depend on scheduling and do not prove the measured interval.
- A shared clock file can be inherited across both processes without changing the wire protocol. Atomic replacement ensures each read sees one complete value.
- A filesystem watcher did not deliver the replacement event on the macOS test host despite registering before the SOCKS greeting. The explicit loopback control connection supplies the wake-up event; it does not set the time itself.

## Boundaries

Only measurement timestamps and the raw SOCKS negotiation deadline use the test clock. Scheduling, liveness and other operational deadlines keep the real clock. A malformed or unreadable configured file fails loudly in the `failpoints` build. Connect and WebSocket liveness timeouts stay as specified.
