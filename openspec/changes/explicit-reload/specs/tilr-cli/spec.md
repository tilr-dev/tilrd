## MODIFIED Requirements

### Requirement: tilr deploy installs and starts tilrd over SSH
`tilr deploy [--service] [--archive FILE] [--cdn URL] <dest>` SHALL use SSH to install a locally checked platform release in remote `~/.local/bin`, optionally install its service, start tilrd, and print attach/forward commands and token. Failures SHALL exit non-zero and restore remote files, except reload request failure after installation: retain both new programs, report installation succeeded but reload could not be requested, and omit success output.

**Verified by**: `client::deploy_requests_reload_without_a_default_server_and_keeps_a_refused_installation`, itests `UC-07C` (`uc_07c.py::deploy_installs_and_starts_tilrd_on_a_host_reached_over_ssh`, `uc_07c.py::a_failed_deploy_leaves_the_remote_as_it_was`) and `client::deploy_names_a_platform_without_a_release`, `client::deploy_through_a_stub_ssh_installs_both_and_prints_how_to_connect`, `client::deploy_rolls_back_both_programs_when_remote_start_or_service_fails`, `client::deploy_gets_the_token_before_replacing_an_old_protocol_client`, `client::deploy_restores_service_configuration_when_the_manager_refuses_it`, `client::deploy_restores_the_crontab_after_a_service_failure`, `client::deploy_does_not_start_a_daemon_before_a_failing_service_install` (crates/tilr/tests/client.rs).

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

#### Scenario: Service failure on a fresh host
- **WHEN** no tilrd runs on the host and its service manager refuses installation
- **THEN** deploy fails without starting a daemon or creating its token, and restores both programs and service configuration

#### Scenario: Reload request fails after installation
- **WHEN** both programs are installed and token retrieval succeeds but the explicit reload request fails
- **THEN** deploy exits non-zero with an installed-but-reload-failed error, keeps both new programs and prints no success instructions or token
