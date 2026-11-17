## MODIFIED Requirements

### Requirement: Command set
`tilr` SHALL offer the subcommands `new`, `attach` (alias `a`), `ls` (alias `list`), `kill`, `kill-server`, `ps`, `profiles`, `bus call|sub|ls|start|stop|reload|reload-config|token|revoke|rotate-key`, `version`, `token`, `cloud login|status|logout`, `update`, `deploy`, `service install|uninstall`, `edit` and `open`, and the global options `--socket PATH`, `--connect URL` and `--token TOKEN` (also `TILR_SOCKET`, `TILR_CONNECT`, `TILR_TOKEN`), speaking `tilr.bus.v1` to the server. With no subcommand it SHALL behave as `attach` with no pane. `edit` and `open` SHALL make no server connection (see `clipboard-host-events`).

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

#### Scenario: profiles and bus are commands
- **WHEN** the user runs `tilr profiles` and `tilr bus ls` on a server with no profiles and no `bus.toml`
- **THEN** both print nothing and exit 0

### Requirement: new
`tilr new [-d] [--cwd DIR] [--profile NAME] [-- CMD…]` SHALL call `pane.create` with the terminal's size (80x24 without one), `command` = the words after `--` (empty: the profile's command or the login shell), `profile` = `NAME` (empty without `--profile`), and `cwd` = `--cwd` made absolute; without `--cwd` it SHALL send the current directory when no `--profile` is given and an empty `cwd` when one is. It SHALL then attach to the new pane; with `-d` it SHALL print the pane's full id on one line, exit 0 and not attach. A refusal SHALL be printed (its message) and exit 1.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::kill_server_ends_the_shells_and_removes_the_socket`, `client::tilr_new_with_a_profile_starts_in_the_profiles_directory`.

#### Scenario: Detached new
- **WHEN** the user runs `tilr new -d -- sleep 1000`
- **THEN** stdout is the pane id and a newline, and `tilr ps` lists that pane with command `sleep 1000`

#### Scenario: Detached new with a profile
- **WHEN** the user runs `tilr new -d --profile demo` in `/tmp`, and `demo` says `cwd = "~"` and `command = ["sleep", "1000"]`
- **THEN** stdout is the pane id, and the pane runs `sleep 1000` in the home directory, not in `/tmp`

#### Scenario: Unknown profile
- **WHEN** the user runs `tilr new -d --profile nope` and there is no such profile
- **THEN** stderr says `create pane: no such profile: nope` and the exit status is 1
