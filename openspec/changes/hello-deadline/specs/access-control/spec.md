## MODIFIED Requirements

### Requirement: A handshake has 10 seconds
A connection SHALL be closed by the server when it has not completed its WebSocket handshake (or, on TCP, sent its first byte, finished its raw SOCKS5 negotiation or its `/metrics` request) within 10 s of being accepted, without an answer. A main-protocol connection (TCP or Unix socket) that has not sent `Hello` within 10 s of completing its WebSocket handshake SHALL be closed with close code 4002 and reason `timeout`, whatever `--client-timeout` is (also 0), and the log SHALL say `client sent no Hello in 10s`. The server MUST NOT otherwise limit the number of connections, and silent ones MUST NOT stop other clients. After `Hello` the client timeout applies instead (see `client-liveness`).

**Verified by**: `auth::a_connection_that_never_says_hello_is_closed_after_10_s`, itests UC-039 (`uc_039.py::a_silent_connection_is_closed_by_the_handshake_timeout`, `uc_039.py::a_websocket_that_never_says_hello_is_closed_after_10_s`) and UC-038 (`uc_038.py::a_thousand_silent_connections_and_a_slowloris_leave_the_server_working`).

#### Scenario: Connect and say nothing
- **WHEN** a client opens a TCP connection to the port and sends nothing
- **THEN** the server closes it about 10 s later

#### Scenario: WebSocket without Hello
- **WHEN** a client completes the WebSocket handshake with a server started with `--client-timeout 0`, answers its pings and sends no `Hello`
- **THEN** the server closes the connection with code 4002 `timeout` about 10 s after the handshake, and the log has `client sent no Hello in 10s`

#### Scenario: Slowloris
- **WHEN** a thousand connections sit silent and one sends its handshake a byte at a time
- **THEN** a normal client still connects and runs commands, and the silent ones are closed after 10 s
