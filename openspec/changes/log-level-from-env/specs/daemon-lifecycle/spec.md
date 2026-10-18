## ADDED Requirements

### Requirement: RUST_LOG sets the log level
The server SHALL take its log filter from `RUST_LOG`, defaulting to `info` when unset or invalid.

#### Scenario: Debug logging
- **WHEN** `tilrd` runs with `RUST_LOG=debug`
- **THEN** its log on stderr contains DEBUG lines
