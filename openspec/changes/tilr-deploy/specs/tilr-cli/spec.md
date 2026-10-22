## ADDED Requirements

### Requirement: tilr deploy installs and starts tilrd over SSH
`tilr deploy [--service] [--archive FILE] [--cdn URL] <dest>` SHALL use the user's `ssh` to find the remote platform, send it the matching release archive checked on the local machine, install `tilrd` and `tilr` into the remote `~/.local/bin` both or neither, run `tilr service install` there with `--service`, start tilrd there, and print an `ssh -t <dest> '~/.local/bin/tilr'` line, an `ssh -N -L` line and the remote token; any failure SHALL exit non-zero leaving the remote files as they were.

**Verified by**: itests `UC-07C` (`uc_07c.py::deploy_installs_and_starts_tilrd_on_a_host_reached_over_ssh`, `uc_07c.py::a_failed_deploy_leaves_the_remote_as_it_was`) and `client::deploy_names_a_platform_without_a_release`, `client::deploy_through_a_stub_ssh_installs_both_and_prints_how_to_connect`, `client::deploy_rolls_back_both_programs_when_remote_start_or_service_fails`, `client::deploy_gets_the_token_before_replacing_an_old_protocol_client`, `client::deploy_restores_service_configuration_when_the_manager_refuses_it`, `client::deploy_restores_the_crontab_after_a_service_failure` (crates/tilr/tests/client.rs).

#### Scenario: A fresh host
- **WHEN** the user runs `tilr deploy user@host` and host runs Linux x86_64 without tilr
- **THEN** `~/.local/bin/tilrd` and `tilr` on host are the release's, a tilrd runs there, and the output ends with the attach line, the forward line and the token

#### Scenario: Local and remote home directories differ
- **WHEN** the user runs the printed terminal command in a local shell whose home differs from the remote home
- **THEN** the command preserves `~/.local/bin/tilr` for expansion by the remote shell

#### Scenario: A damaged archive
- **WHEN** the archive given with `--archive` does not match its `.sha256`
- **THEN** nothing is sent to host and the command exits non-zero with `checksum mismatch`

#### Scenario: No release for the platform
- **WHEN** host runs a platform the CDN has no `latest` for and no `--archive` is given
- **THEN** the command exits non-zero naming the platform and `--archive`, and host is unchanged

#### Scenario: A server already running there
- **WHEN** tilrd already runs on host with a pane, and a newer release is deployed
- **THEN** that tilrd reloads into it and the pane's shell is the same process

#### Scenario: An upgrade changes the wire protocol
- **WHEN** the installed client can talk to the running server and the new release uses a different protocol
- **THEN** deploy obtains the persistent token through the installed client before replacing it, and succeeds without a new client handshake with the old server

#### Scenario: Service installation fails
- **WHEN** `--service` fails after writing its configuration
- **THEN** deploy restores the previous service configuration and both programs, exits non-zero and prints no connection commands

### Requirement: Internal release fetching for deploy
`tilrd fetch-release --platform P --out DIR [--cdn URL] [--archive FILE]` SHALL validate a release's checksum and both regular program members using the updater's checks, write its archive into DIR, and exit non-zero on failure. It SHALL remain hidden from help and SHALL NOT execute the programs. With `--archive`, it SHALL validate the local archive and adjacent `.sha256` without HTTP.

**Verified by**: `update::fetch_release_writes_a_checked_archive`, `update::fetch_release_refuses_a_bad_checksum` and `client::deploy_refuses_a_damaged_archive_before_sending_it`.

#### Scenario: Fetch a foreign platform
- **WHEN** a user fetches a valid release for a platform other than the local one
- **THEN** the checked archive is written without running either program

#### Scenario: Bad checksum
- **WHEN** a downloaded or supplied archive does not match its checksum
- **THEN** the command exits non-zero without writing a release archive


## MODIFIED Requirements

### Requirement: Command set
`tilr` SHALL offer the subcommands `new`, `attach` (alias `a`), `ls` (alias `list`), `kill`, `kill-server`, `ps`, `version`, `token`, `cloud login|status|logout`, `update`, `deploy`, `service install|uninstall`, `edit` and `open`, and the global options `--socket PATH`, `--connect URL` and `--token TOKEN` (also `TILR_SOCKET`, `TILR_CONNECT`, `TILR_TOKEN`). With no subcommand it SHALL behave as `attach` with no pane. `edit` and `open` SHALL make no server connection (see `clipboard-host-events`).

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::tilr_edit_in_a_pane_asks_the_server_of_the_pane`.

#### Scenario: No subcommand
- **WHEN** the user runs `tilr` in a terminal
- **THEN** it attaches to the pane used last, or makes one and attaches to it

#### Scenario: Version flag
- **WHEN** the user runs `tilr --version`
- **THEN** it prints `tilr <version> (<platform>)`

#### Scenario: edit needs no server connection
- **WHEN** `tilr edit FILE` runs in a pane
- **THEN** the request goes over `$TILR_SOCK` as the `clipboard-host-events` spec says, and no WebSocket connection to tilrd is made
