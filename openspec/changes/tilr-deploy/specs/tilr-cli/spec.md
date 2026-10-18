## ADDED Requirements

### Requirement: tilr deploy installs and starts tilrd over SSH
`tilr deploy [--service] [--archive FILE] [--cdn URL] <dest>` SHALL use the user's `ssh` to find the remote platform, send it the matching release archive checked on the local machine, install `tilrd` and `tilr` into the remote `~/.local/bin` both or neither, run `tilr service install` there with `--service`, start tilrd there, and print an `ssh -t <dest> ~/.local/bin/tilr` line, an `ssh -N -L` line and the remote token; any failure SHALL exit non-zero leaving the remote files as they were.

**Verified by**: itests `UC-07C` (`uc_07c.py::deploy_installs_and_starts_tilrd_on_a_host_reached_over_ssh`, `uc_07c.py::a_failed_deploy_leaves_the_remote_as_it_was`) and `client::deploy_names_a_platform_without_a_release` (crates/tilr/tests/client.rs).

#### Scenario: A fresh host
- **WHEN** the user runs `tilr deploy user@host` and host runs Linux x86_64 without tilr
- **THEN** `~/.local/bin/tilrd` and `tilr` on host are the release's, a tilrd runs there, and the output ends with the attach line, the forward line and the token

#### Scenario: A damaged archive
- **WHEN** the archive given with `--archive` does not match its `.sha256`
- **THEN** nothing is sent to host and the command exits non-zero with `checksum mismatch`

#### Scenario: No release for the platform
- **WHEN** host runs a platform the CDN has no `latest` for and no `--archive` is given
- **THEN** the command exits non-zero naming the platform and `--archive`, and host is unchanged

#### Scenario: A server already running there
- **WHEN** tilrd already runs on host with a pane, and a newer release is deployed
- **THEN** that tilrd reloads into it and the pane's shell is the same process
