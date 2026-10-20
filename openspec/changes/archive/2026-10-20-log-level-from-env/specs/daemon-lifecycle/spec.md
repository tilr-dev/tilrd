## ADDED Requirements

### Requirement: Log filter from RUST_LOG
`tilrd` (the supervisor and the pane processes) SHALL take its log filter from `RUST_LOG` in tracing's directive syntax, and SHALL log at `info` when `RUST_LOG` is unset or empty. An invalid `RUST_LOG` SHALL give `info` and one line `RUST_LOG ignored: <reason>`.

**Verified by**: `local::rust_log_sets_the_log_level`.

#### Scenario: Debug logging
- **WHEN** `tilrd` runs with `RUST_LOG=debug` and a connection to its Unix socket sends bytes that are not a WebSocket handshake
- **THEN** its stderr has a DEBUG line `websocket handshake failed`
- **AND** without `RUST_LOG` that line does not appear

#### Scenario: Invalid RUST_LOG
- **WHEN** `tilrd` runs with `RUST_LOG='tilrd=[bad'`
- **THEN** it starts, logs `RUST_LOG ignored:` once and logs at `info`
