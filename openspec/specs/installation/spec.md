# installation Specification

## Purpose

Covers the installer (`curl -fsSL https://tilr.sh | sh`, which serves `scripts/install.sh`) and the manual install the README describes: which files it fetches and verifies, where it puts `tilrd` and `tilr`, what it does when a release cannot be installed, running it again over an installation, that it sets up no service, and the first run. Updating with `tilrd update` is the `update` spec; `tilr service install` and the daemon's directories are the `tilr-cli` and `daemon-lifecycle` specs.

## Requirements

### Requirement: Both executables go into ~/.local/bin without root
The installer SHALL put `tilrd` and `tilr` of the latest release into `$TILR_BIN_DIR` (default `~/.local/bin`), mode 0755, owned by the user, without root or `sudo`, changing nothing in `/usr/local/bin` or `/etc` and leaving nothing in `/tmp`, print `downloading tilr <version> (linux <arch>)` and `installed tilrd and tilr <version> in <bin dir>` on stderr, and exit 0.

**Verified by**: itests `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`).

#### Scenario: Fresh install
- **WHEN** a user without root runs the installer with `~/.local/bin` on `PATH`
- **THEN** `~/.local/bin/tilrd` and `~/.local/bin/tilr` are the archive's files (same SHA-256), 0755 and the user's, each prints the version with `--version`, and the only new entries in the home are `~/.local`, `~/.local/bin` and the two executables

#### Scenario: tilr starts the installed tilrd
- **WHEN** the installed `tilr new -d` runs afterwards
- **THEN** exactly one `tilrd` runs, from `~/.local/bin/tilrd`, as the user, and `tilr version` names both versions

### Requirement: The installer reads the CDN layout
The installer SHALL fetch `<cdn>/releases/linux-<arch>/latest`, then `releases/<version>/tilr-<version>-linux-<arch>.tar.gz.sha256` and the archive `releases/<version>/tilr-<version>-linux-<arch>.tar.gz`, and nothing else of the release, with `curl` or else `wget`; the CDN is `$TILR_CDN`, default `https://cdn.tilr.dev`. A `latest` that is empty or has characters other than letters, digits, `.`, `+`, `_`, `-` SHALL fail with `<cdn> names a release that is not a version, nothing installed`.

**Verified by**: itests `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`).

#### Scenario: Archive and checksum requested
- **WHEN** the installer runs against a CDN that logs requests
- **THEN** the requests under `/releases/<version>/` are exactly the archive's `.sha256` and then the archive

### Requirement: The release is checked in a temporary directory first
Before it makes `$TILR_BIN_DIR` or writes in it, the installer SHALL, in a `mktemp -d` directory, download the archive, match it against its `.sha256`, unpack `tilrd` and `tilr` (regular files) and run each with `--version`. Failures: `download failed`, `checksum mismatch`, `the archive of <version> is damaged or lacks tilrd or tilr`, `<name> <version> does not run on this system (it needs glibc 2.36 or newer)`, each ending `, nothing installed`.

**Verified by**: itests `UC-004` (`uc_004.py::a_bad_checksum_installs_nothing`, `uc_004.py::a_download_cut_short_installs_nothing`, `uc_004.py::an_archive_cut_short_installs_nothing`).

#### Scenario: Bad checksum
- **WHEN** the archive's `.sha256` does not match the archive
- **THEN** the installer exits non-zero saying `checksum mismatch, nothing installed` and no server is started

#### Scenario: Archive cut short
- **WHEN** the archive is cut in half and its checksum made of what is left
- **THEN** the installer exits non-zero saying `the archive of <version> is damaged or lacks tilrd or tilr, nothing installed`

### Requirement: A release that cannot be installed leaves nothing
When the release fails any check, the installer SHALL exit non-zero and leave the home and `/tmp` exactly as they were: no executable, no temporary file and no directory (`~/.local`, `~/.local/bin`) made for it; the temporary directory and any staged file SHALL be removed on every exit, an interrupt included.

**Verified by**: itests `UC-004` (`uc_004.py::a_bad_checksum_installs_nothing`, `uc_004.py::a_download_cut_short_installs_nothing`, `uc_004.py::an_archive_cut_short_installs_nothing`, `uc_004.py::an_archive_without_tilr_installs_nothing`, `uc_004.py::a_program_that_does_not_run_installs_nothing`, `uc_004.py::no_release_for_the_platform_installs_nothing`, `uc_004.py::a_latest_version_without_its_archive_installs_nothing`).

#### Scenario: Download cut short
- **WHEN** the connection drops halfway through the archive
- **THEN** the installer exits non-zero saying `download failed` and neither the home nor `/tmp` has anything new, not even an empty `~/.local/bin`

### Requirement: Both programs are installed or neither
A release whose archive lacks `tilr` or `tilrd`, or one of whose programs does not run, SHALL install neither: the installer exits non-zero and does not leave a new `tilrd` behind.

**Verified by**: itests `UC-004` (`uc_004.py::an_archive_without_tilr_installs_nothing`, `uc_004.py::a_program_that_does_not_run_installs_nothing`).

#### Scenario: tilr is not a program
- **WHEN** the archive's `tilr` is not an executable for this system while its `tilrd` is
- **THEN** the installer exits non-zero saying `tilr <version> does not run on this system`, and neither `tilrd` nor `tilr` is installed

### Requirement: No release for the platform is a clear error
The installer SHALL support Linux only (`only Linux is supported for now, see https://tilr.dev`) on `x86_64`/`amd64` and `aarch64`/`arm64` (`no release for <machine>` otherwise). Without `releases/linux-<arch>/latest` it SHALL fail with `no release for linux <arch> at <cdn>`, and a version whose archive checksum is not there with `no release <version> for linux <arch> at <cdn>, nothing installed`, installing nothing.

**Verified by**: itests `UC-004` (`uc_004.py::no_release_for_the_platform_installs_nothing`, `uc_004.py::a_latest_version_without_its_archive_installs_nothing`).

#### Scenario: Only another platform published
- **WHEN** the CDN has `latest` for `linux-riscv64` only
- **THEN** the installer exits non-zero saying `no release for linux <arch>` and the home is unchanged

### Requirement: Installing the same release again changes nothing
When the installed `tilrd` and `tilr` both report the version `latest` names (`--version`), the installer SHALL print `tilr <version> is already installed in <bin dir>`, download nothing, leave both files untouched (same inode and mtime) and exit 0; the running server SHALL not reload.

**Verified by**: itests `UC-003` (`uc_003.py::installing_the_same_release_again_changes_nothing`).

#### Scenario: Same release again
- **WHEN** the installer runs a second time with the same release and a server is running
- **THEN** no archive is requested, both files keep their inode and mtime, the server pid is the same and its log has no `reloading`

### Requirement: A new release reloads the running server
Run with a new release, the installer SHALL rename the new `tilr` into place first and the new `tilrd` over the old one last, so that the running server reloads into it in place (same pid, panes, shells and their state; see the `reload` spec), and SHALL say `the running tilrd reloads itself and keeps its shells` when a `tilrd` of the user runs.

**Verified by**: itests `UC-003` (`uc_003.py::installing_a_new_release_reloads_the_running_server_and_keeps_its_shells`).

#### Scenario: New release over a running server
- **WHEN** a newer release is published and the installer runs
- **THEN** stderr says `downloading tilr <new version>` and `the running tilrd reloads itself and keeps its shells`, both executables report the new version, and the server pid, panes, `$KEPT`, `$$` and background job are the same

### Requirement: A failed update leaves the installation and its server alone
When an installation exists and the new release fails a check, the installer SHALL exit non-zero (`checksum mismatch, nothing installed`, `... lacks tilrd or tilr, nothing installed`), leave the home's files as they were and the running server (same pid and version) alone. If the new `tilrd` cannot be renamed into place after `tilr` was, it SHALL put the old `tilr` back (or remove the new one if there was none).

**Verified by**: itests `UC-004` (`uc_004.py::a_broken_update_leaves_the_installation_and_its_server_alone`).

#### Scenario: Bad checksum over a running install
- **WHEN** a next release with a bad checksum, then one without `tilr`, is published over a running installation
- **THEN** the installer fails each time, nothing in the home changes outside `~/.local/state`, and `tilr version` still reports the old versions from the same server pid

### Requirement: PATH is advised, never edited
When `$TILR_BIN_DIR` is not on `PATH`, the installer SHALL say `<bin dir> is not on your PATH; add this line to ~/.profile and log in again:` followed by the line `  export PATH="$HOME/.local/bin:$PATH"` (the directory under `$HOME` written as `$HOME/...`), and SHALL NOT edit any file of the user's (`~/.profile`, shell rc files) or in `/etc`.

**Verified by**: itests `UC-002` (`uc_002.py::without_local_bin_on_path_the_installer_says_what_to_add_and_edits_nothing`).

#### Scenario: Bin dir not on PATH
- **WHEN** the installer runs with a `PATH` that lacks `~/.local/bin`
- **THEN** it exits 0, prints the ready line, the line puts `tilr` on `PATH`, and only the executables and their directories are new in the home

### Requirement: The installer sets up no service
The installer SHALL NOT write a systemd unit, crontab line, launchd agent or anything else that starts `tilrd`, and SHALL NOT call `systemctl`, `loginctl`, `crontab` or `sudo`, even where a systemd user manager is there. It SHALL end with the hints `open a shell: tilr`, `start tilrd at boot: tilr service install`, `the access token for the app: tilr token` and `link this machine to Tilr Cloud: tilr cloud login`.

**Verified by**: itests `UC-023` (`uc_023.py::nothing_starts_tilrd_at_boot_unless_asked`), `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`).

#### Scenario: systemd is there
- **WHEN** the installer runs where a systemd user manager answers and lingering is off
- **THEN** it exits 0, says `start tilrd at boot: tilr service install`, and `systemctl`, `loginctl` and `crontab` were not called

### Requirement: Without systemd the install still works
Without systemd, root or `sudo` the installer SHALL exit 0 and `tilr` SHALL then start the server and everything SHALL work.

**Verified by**: itests `UC-006` (`uc_006.py::without_root_sudo_or_systemd_everything_works`).

#### Scenario: No systemd, no sudo
- **WHEN** an unprivileged user installs in a container without systemctl, with a `sudo` that records being called
- **THEN** the installer exits 0 with the hints, `tilr new -d`, `tilr ls`, `tilr token` and a TCP client on 127.0.0.1:7890 work, and `sudo` was never called

### Requirement: The first run makes private directories
After an install on a clean system (no `$XDG_RUNTIME_DIR`, no `~/.config`), the first `tilr` SHALL start `tilrd`, which makes `/tmp/tilr-<uid>` (with `tilrd.sock`, `tilrd.sock.lock`, `tilrd.log`) and `~/.local/state/tilr` (with `token`) private: directories 0700, files and sockets 0600; see the `daemon-lifecycle` spec.

**Verified by**: itests `UC-005` (`uc_005.py::on_a_clean_system_tilr_makes_its_directories_private`).

#### Scenario: Clean system
- **WHEN** the user installs, runs `tilr new -d` and `tilr token`
- **THEN** `/tmp/tilr-<uid>` and everything in it are 0700/0600, `~/.local/state/tilr` is 0700 and `token` is 0600 holding the printed token

### Requirement: Manual install from the CDN
A manual install SHALL need only the CDN layout: read `releases/<platform>/latest`, download `releases/<version>/tilr-<version>-<platform>.tar.gz`, unpack `tilrd` and `tilr` from it into `~/.local/bin`, and run `tilr`.

**Verified by**: itests `UC-007` (`uc_007.py::a_manual_install_from_the_cdn_layout_runs`).

#### Scenario: By hand
- **WHEN** a user fetches `latest` for `linux-aarch64` and unpacks `tilrd` and `tilr` of that version's archive into `~/.local/bin`
- **THEN** `tilr` starts the unpacked `tilrd` and attaches

## Notes

- Requirements without a test: the old-`tilr` restore of "A failed update leaves the installation and its server alone" (no case makes the `tilrd` rename fail).
- Since 3ba0e93 the installer checks the whole release in a temporary directory before it makes `~/.local/bin`; UC-004 (all its cases) is green. The earlier installer's service setup (`tilr.service`, linger, `TILR_NO_SERVICE`), its move of a `tilr-server` install to `tilrd` and its client-less releases are gone with it, along with the loose per-program files of the CDN (2a8b425).
- The installer is `scripts/install.sh`, the one the itests run; `scripts/release.sh` uploads it to R2 (`https://cdn.tilr.dev/install.sh`, served by `tilr.sh`) after a release's `latest` (`release-process` spec). The `tilr.sh` Worker lives outside this repo.
- No known red itests case for installation.
