## MODIFIED Requirements

### Requirement: Foreground service with its log on stderr
`tilrd` with no subcommand SHALL run in the foreground until stopped and SHALL write its log to stderr, with colors only when stderr is a terminal and `NO_COLOR` is unset or empty. Its log filter SHALL be `RUST_LOG` (tracing directive syntax), `info` when unset or empty; an invalid `RUST_LOG` SHALL give `info` and one line `RUST_LOG ignored: <reason>`. Once it listens it SHALL log `listening on ws://<addr>` for the TCP port and `Unix socket for local clients` with the socket's path. An error that stops it at start SHALL end it with exit status 1 and the error on stderr; a stop on SIGTERM SHALL exit 0.

**Verified by**: itests `UC-016` (`uc_016.py::tilr_attaches_to_a_tilrd_running_in_the_foreground`), `local::a_server_that_stops_removes_its_socket_and_lock`, `local::rust_log_sets_the_log_level`.

#### Scenario: Run by hand
- **WHEN** the user runs `tilrd` with a clean environment and then `tilr`
- **THEN** stderr says `Unix socket for local clients` and `listening on ws://127.0.0.1:7890`, and `tilr` attaches to this tilrd without starting another
- **AND** SIGTERM makes tilrd exit 0 and the attached `tilr` print `[tilrd stopped]`

#### Scenario: Debug logging
- **WHEN** `tilrd` runs with `RUST_LOG=debug` and a connection to its Unix socket sends bytes that are not a WebSocket handshake
- **THEN** its stderr has a DEBUG line `websocket handshake failed`
- **AND** without `RUST_LOG` that line does not appear

#### Scenario: Invalid RUST_LOG
- **WHEN** `tilrd` runs with `RUST_LOG='tilrd=[bad'`
- **THEN** it starts, logs `RUST_LOG ignored:` once and logs at `info`
