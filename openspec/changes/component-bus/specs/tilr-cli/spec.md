## MODIFIED Requirements

### Requirement: Command set
`tilr` SHALL offer the subcommands `new`, `attach` (alias `a`), `ls` (alias `list`), `kill`, `kill-server`, `ps`, `profiles`, `bus call|ls|start|stop|reload|reload-config`, `version`, `token`, `cloud login|status|logout`, `update`, `deploy`, `service install|uninstall`, `edit` and `open`, and the global options `--socket PATH`, `--connect URL` and `--token TOKEN` (also `TILR_SOCKET`, `TILR_CONNECT`, `TILR_TOKEN`). With no subcommand it SHALL behave as `attach` with no pane. `edit` and `open` SHALL make no server connection.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::tilr_edit_in_a_pane_asks_the_server_of_the_pane`, `client::tilr_profiles_lists_the_profiles_of_the_server`, `client::tilr_bus_call_prints_the_reply`.

#### Scenario: No subcommand
- **WHEN** the user runs `tilr` in a terminal
- **THEN** it attaches to the pane used last, or makes one and attaches to it

#### Scenario: Version flag
- **WHEN** the user runs `tilr --version`
- **THEN** it prints `tilr <version> (<platform>)`

#### Scenario: edit needs no server connection
- **WHEN** `tilr edit FILE` runs in a pane
- **THEN** the request goes over `$TILR_SOCK` as the `clipboard-host-events` spec says, and no WebSocket connection to tilrd is made

#### Scenario: bus is a command
- **WHEN** the user runs `tilr bus ls` on a server without `bus.toml`
- **THEN** it prints nothing and exits 0
