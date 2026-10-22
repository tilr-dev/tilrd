# update Specification

## Purpose

Covers `tilrd update` and the user's `tilr update`: finding the latest release for the platform on the CDN, downloading its archive (`tilrd` and `tilr` together) and verifying it, putting both programs in place so that a running server reloads into the new `tilrd`, and every way an update can fail without changing anything. The reload itself is the `reload` spec; how releases get onto the CDN is the `release-process` spec; the first install is the `installation` spec.

## Requirements

### Requirement: The latest release is read from the CDN
`tilrd update` SHALL read `<cdn>/releases/<platform>/latest` (the platform as `--version` names it) and SHALL refuse a trimmed value that is empty or has characters other than ASCII letters, digits, `.`, `-`, `+`, `_` (`... names a release that is not a version: ...`). It SHALL then fetch `releases/<version>/tilr-<version>-<platform>.tar.gz.sha256` and download `releases/<version>/tilr-<version>-<platform>.tar.gz`, printing `downloading tilr <version> (<platform>)`.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves`, `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: A login page instead of the CDN
- **WHEN** `releases/<platform>/latest` answers `<html><body>Sign in</body></html>`
- **THEN** the update exits non-zero with `not a version` on stderr and the executable is unchanged

### Requirement: The CDN is configurable
The CDN SHALL be `https://cdn.tilr.dev` unless `--cdn URL` or the environment variable `TILR_CDN` names another; a trailing `/` SHALL be ignored.

**Verified by**: `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: A CDN of the test's own
- **WHEN** `tilrd update --cdn http://127.0.0.1:<port>` runs
- **THEN** every request goes to that base URL

### Requirement: An up-to-date install does nothing
When `latest` equals the running build's version, `tilrd update` SHALL print `tilrd <version> (<platform>) is the latest release`, download nothing, leave the executable untouched and exit 0. When the installed `tilrd` and the `tilr` next to it are already byte for byte the archive's two programs, it SHALL print `tilr <version> (<platform>) is already installed`, change nothing and exit 0.

**Verified by**: `update::update_does_nothing_when_the_latest_release_is_the_one_installed`, `update::two_updates_at_once_install_the_release_once`.

#### Scenario: Latest is installed
- **WHEN** the CDN names the running version as latest and has no release files
- **THEN** stdout says `is the latest release`, the exit status is 0, and the executable's mtime is unchanged

### Requirement: The download must match its checksum
The archive SHALL be at most 256 MiB and SHALL match the first word of its `.sha256` file (hex SHA-256, case-insensitive); otherwise the update SHALL fail with `the download does not match its checksum, nothing was installed`, leaving the executable as it was and nothing next to it.

**Verified by**: `update::update_installs_nothing_that_fails_its_checksum_or_does_not_run`.

#### Scenario: Tampered archive
- **WHEN** the archive on the CDN is not the file its checksum was made of
- **THEN** stderr says `does not match its checksum`, the exit status is non-zero, and the directory holds only the old executable

### Requirement: The archive must hold both programs
The verified archive (gzip tar) SHALL be unpacked into a new directory `.tilr-update.<pid>` (mode 0700) next to the executable, taking only the top-level entries `tilrd` and `tilr`, each a regular file of at most 512 MiB, written 0755. An archive that does not unpack fails with `unpack release <version>, nothing was installed`; one without either program with `the archive has no tilr` / `the archive has no tilrd`. Either way nothing is installed and the directory is removed.

**Verified by**: `update::update_installs_nothing_that_fails_its_checksum_or_does_not_run`.

#### Scenario: Archive without the client
- **WHEN** the release archive holds `tilrd` only, its checksum matching
- **THEN** stderr says `the archive has no tilr`, the exit status is non-zero, and the directory holds only the old executable

### Requirement: Both programs must run before either is installed
Each unpacked program SHALL be run as `<staged program> --version` and the release installed only if both exit 0; otherwise the update SHALL fail with `<tilr|tilrd> <version> does not run on this system, nothing was installed`. A release missing for the platform SHALL also fail and install nothing.

**Verified by**: `update::update_installs_nothing_that_fails_its_checksum_or_does_not_run`.

#### Scenario: The client is not a program
- **WHEN** the archive is intact but its `tilr` is not an executable for this system
- **THEN** stderr says `tilr <version> does not run on this system` and neither program is installed

### Requirement: The release is put in place by two renames, the server last
The update SHALL rename the staged client into place first and the server last, preserving the executable's installed name, then explicitly request reload from that installation. It SHALL NOT start a server. On success it SHALL print `updated <old> -> <new>: <directory>` and `a running server reloads itself and keeps its shells`, exit 0 and leave only the installed programs.

**Verified by**: `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: Installed destinations
- **WHEN** the installed executable is named `custom-server`
- **THEN** update renames the unpacked `tilr` over `tilr` in that directory first, then the unpacked `tilrd` over `custom-server`

#### Scenario: Update with a pane open
- **WHEN** a server with a pane holding `KEPT=yes` runs `tilrd update` against a CDN with a newer release
- **THEN** stdout says `-> 9.9.9-test`, the executable and the `tilr` next to it are the release's, the server logs an explicit reload request, and after reconnecting `test "$KEPT" = yes` exits 0

### Requirement: The installed release is on disk before success
Both unpacked programs SHALL be synced to disk before the first rename and the executable's directory after the second, before the update prints `updated`.

**Verified by**: itests `UC-060` (`uc_060.py::an_update_is_synced_before_it_reports_success`).

#### Scenario: Synced before success
- **WHEN** `tilrd update` installs a newer release and its system calls are traced
- **THEN** each unpacked program is `fsync`ed before the first `rename`, the executable's directory is `fsync`ed after the second, and both before `updated` is printed

### Requirement: A server that cannot be put in place puts the old client back
If the rename of `tilrd` fails after `tilr` was put in place, the update SHALL put the previous `tilr` back (or remove the new one if there was none), leave the executable as it was and fail with `... nothing was installed`.

**Verified by**: `update::a_server_that_cannot_be_put_in_place_puts_the_old_client_back`.

#### Scenario: The server's rename fails
- **WHEN** an update fails at the failpoint `update-between-renames` with an old `tilr` next to the executable
- **THEN** stderr says `nothing was installed`, the executable and the old `tilr` are as they were, and nothing else is left in the directory

### Requirement: Live programs survive an update
An update that the running server reloads into SHALL keep the server's pid and every program in its panes alive and answering, with their screens, and leave no orphan or extra descriptor; the `tilr` next to the executable SHALL be the new version too.

**Verified by**: itests `UC-060` (`uc_060.py::update_from_the_cdn_keeps_every_program_in_the_panes_alive`), `UC-063` (`uc_063.py::a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled`).

#### Scenario: Update with vim, htop and a REPL
- **WHEN** `tilrd update` installs a newer release while the panes run a shell, vim, htop, less, python and a TCP client
- **THEN** the server reports the new version with the same pid, `tilr --version` next to it names the new version, and every program answers as before

### Requirement: A CDN that misbehaves installs nothing
A download cut short, release files missing after `latest` names them (no `.sha256`: `<cdn> has no release <version> for <platform>`), or a CDN that does not answer SHALL make the update exit non-zero (`download ...` for a cut download) and leave the executable unchanged with nothing next to it.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves`.

#### Scenario: Download cut short
- **WHEN** the connection breaks off halfway through the archive
- **THEN** stderr mentions `download`, the exit status is non-zero, and the directory holds only the old executable

### Requirement: An unwritable directory installs nothing
When the update cannot write next to the executable, it SHALL fail (`write ...` on stderr) and leave the executable as it was.

**Verified by**: `update::update_installs_nothing_when_it_cannot_write_next_to_the_executable`.

#### Scenario: Read-only directory
- **WHEN** the executable's directory is mode 0555 for a non-root user
- **THEN** the update fails with `write` on stderr and the directory holds only the old executable

### Requirement: A killed update changes nothing and is tidied up
An update killed before its renames SHALL leave the executable (and `tilr`) as they were; the next update SHALL remove any `.tilr-update.*` directory left (with the lock held) and install the release.

**Verified by**: `update::an_update_killed_before_the_swap_changes_nothing_and_the_next_one_tidies_up`.

#### Scenario: Killed before the swap
- **WHEN** an update is killed at the failpoint `update-before-rename` with the release unpacked on disk
- **THEN** the executable is unchanged; the next update succeeds and leaves only `tilr` and `tilrd` in the directory

### Requirement: A disk full during the download installs nothing
An update that runs out of space unpacking the release SHALL fail saying so (the error contains `space`), leave the executable as it was with nothing beside it, and the server SHALL not reload; with room again the same update SHALL succeed and install both `tilrd` and `tilr`.

**Verified by**: itests `UC-065` (`uc_065.py::an_update_that_runs_out_of_disk_while_downloading_installs_nothing_and_breaks_nothing`).

#### Scenario: Small disk
- **WHEN** the executable's filesystem has room for little more than one executable and `tilrd update` runs
- **THEN** it exits non-zero mentioning space, the directory holds only `tilrd`, and the server logs no `reloading`

### Requirement: One update at a time
An update SHALL hold an exclusive `flock` on the executable's directory from after it has read the checksum, before it downloads, until it ends; a second update SHALL wait and then find the release `already installed`, so it is installed once.

**Verified by**: `update::two_updates_at_once_install_the_release_once`.

#### Scenario: Two at once
- **WHEN** two updates of the same executable run concurrently, the first slowed before its rename
- **THEN** both exit 0, one prints `-> 9.9.9-test`, the other `is already installed`, and only `tilr` and `tilrd` are left

### Requirement: Rolling back is an update to an older release
`tilrd update` SHALL install whatever version `latest` names, older or newer, by the same archive, checks and renames; a rollback and the update forward again SHALL each be a reload with the server's pid and panes kept.

**Verified by**: itests `UC-066` (`uc_066.py::rolling_back_and_forward_again_keeps_the_panes_and_their_programs`).

#### Scenario: Back and forward
- **WHEN** a server updated to the next release finds `latest` naming its previous version, then the next one again
- **THEN** after each update the server and the `tilr` next to it report that version, with the same pid and panes

### Requirement: Only the release archive is looked for
`tilrd update` SHALL look for a release only as `tilr-<version>-<platform>.tar.gz`; it SHALL NOT fall back to the loose files of releases from before the archive (`tilrd-<platform>`, `tilr-server-<platform>`), and a version published only that way SHALL fail with `<cdn> has no release <version> for <platform>`. **BREAKING** (2a8b425): a server installed from before the archive, which asks for the loose files, finds no new release and is reinstalled once with the installer.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves` (the `has no release 9.9.9-test` step).

#### Scenario: Old layout only
- **WHEN** the CDN's `latest` names a version that has no `tilr-<version>-<platform>.tar.gz.sha256`
- **THEN** the update exits non-zero with `has no release <version>` and the directory holds only the old executable

### Requirement: A server managed by Tilr.app is not updated here
When `app-managed.json` is next to the executable, `tilrd update` SHALL fail before contacting any CDN, saying the server `is managed by Tilr.app` and to use Tilr > Check for Server Updates.

**Verified by**: `update::app_managed_server_never_contacts_the_standalone_update_feed`.

#### Scenario: App-managed copy
- **WHEN** `app-managed.json` is next to the executable and `tilrd update --cdn http://127.0.0.1:1` runs
- **THEN** it exits non-zero with `managed by Tilr.app` on stderr

### Requirement: tilr update runs the update in the local server
`tilr update [--cdn URL]` (or `TILR_CDN`) SHALL run `tilrd update [--cdn URL]` in the local server as a server command (a child of the server, in its environment; see the `exec` spec) and SHALL refuse a remote target with `server commands run in the local tilrd; on another machine, run tilr there`.

**Verified by**: `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload`.

#### Scenario: Remote target
- **WHEN** `tilr --connect ws://host:7890 update` runs
- **THEN** it fails with `server commands run in the local tilrd; on another machine, run tilr there`

### Requirement: tilr update waits for the reloaded server
When the update command exits 0, or ends with reason `reload` because the server reloaded into it, `tilr update` SHALL reconnect for up to 15 s and print `tilrd <version> is running`, else fail with `the server did not come back after the update: ...`. Another exit code SHALL be `tilr`'s exit status.

**Verified by**: `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload`.

#### Scenario: Update from a shell
- **WHEN** the user runs `tilr update` and a newer release is installed
- **THEN** the server reloads and `tilr` prints `tilrd <new version> is running` and exits 0

### Requirement: Update command waiting is independent of launch latency
The client SHALL wait for the local update command to complete even when staged executable startup takes longer than the reload reconnect window. The command SHALL have no execution timeout; reconnect observation begins after command completion or reload.

**Verified by**: `conn::update_wait_tests::update_waits_for_command_exit_after_a_long_launch_delay`, `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload`.

#### Scenario: Staged startup outlasts the reload window
- **WHEN** a local update command has started and a day of virtual time passes before its successful exit
- **THEN** the client remains waiting, requests no execution timeout, and returns success only after receiving the command exit

#### Scenario: Integration observation follows actual staged startup
- **WHEN** the integration harness updates with real release executables whose staged version checks have not yet completed
- **THEN** it waits for the updater's installation confirmation without a launch deadline before starting the unchanged reload observation window

## Notes

- `tilr update` from the client is run by `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload` against a local CDN directory; the itests run `tilrd update` directly.
- The one-archive release replaced the loose files `tilrd-<platform>`, `tilr-server-<platform>` and `tilr-<platform>` in 2a8b425; the dual-name transition (the rename from `tilr-server`, and its itests case, removed in 3ba0e93) went with it, without compatibility for old updaters (owner's decision, 2026-10-18).
- `scripts/update-test.sh HOST OLD-VERSION` is a manual live check against the real CDN (install by hand from the archive, update from a pane, again, no CDN, rollback, stopped server); it is not run in CI.
- Also covered by `uc_004.py::a_broken_update_leaves_the_installation_and_its_server_alone` (the installer's update path, see the `installation` spec).
- No known red itests case for update.
