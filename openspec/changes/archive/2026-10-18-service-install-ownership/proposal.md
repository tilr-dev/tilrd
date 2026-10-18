# Proposal: tilr service install leaves foreign unit files alone; the state directory is made private

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (findings #23, #24). Re-checked at 1163abd:
- `tilr service install` overwrites a `tilr.service` or `sh.tilr.tilrd.plist` it did not write: `install_systemd` writes the unit with no marker check (`crates/tilr/src/service.rs:147-151`), `install_launchd` the plist likewise (`service.rs:356-362`). Uninstall already checks the marker (`service.rs:176-182`, `398-404`; spec "Service uninstall removes only its own", UC-022 `uninstall_leaves_what_it_did_not_write`), so that half of #23 is fixed; the crontab path keeps every line that is not its own (`service.rs:228-233`, `is_ours` `276-278`).
- Crontab detection matches the English `no crontab for` (`service.rs:290-293`); `crontab -l` without a crontab says something else elsewhere (busybox: `can't open '…': No such file or directory`), and install then fails.
- `write_private` writes 0644 (`service.rs:454-470`) although its comment says "readable by the user only"; the `tilr-cli` spec states 0644 for the unit, so only the name and comment are wrong (no behavior change).
- An existing state directory is not tightened to 0700: `create_private_dir` creates it 0700 but accepts an existing directory as it is (`crates/tilrd/src/identity.rs:28-38`), and `read_or_create` calls it only when `server-id` is missing (`identity.rs:52-55`), so a `~/.local/state/tilr` left 0755 stays 0755 with the token in it (finding #24).

## What Changes

- `tilr service install` refuses to replace a unit or plist that does not carry its marker (`tilr service`): it exits non-zero with `<path> was not written by `tilr service install`: move it away to install` and changes nothing (no `systemctl`, no `launchctl`).
- `crontab -l` runs with `LC_ALL=C`; "no crontab yet" is recognized from cronie's/Vixie's `no crontab for` and busybox's `No such file or directory`; any other failure still stops install.
- `write_private` is renamed to say what it does (written whole, 0644); no mode change.
- At start the server sets an existing state directory of its own to 0700 and refuses one owned by another user (`<dir> belongs to another user`, as for the runtime directory), before it reads or writes `server-id`, the token or the Cloud link.

## Capabilities

### Modified Capabilities
- `tilr-cli`: new requirements "Service install leaves a unit or plist it did not write" and "A missing crontab is recognized whatever the cron".
- `daemon-lifecycle`: new requirement "An existing state directory is made private".

Changed while implementing: the behaviour was first written into the existing requirements ("Service install with systemd", "… on macOS", "… with cron", "State directory"); that made their text longer than `openspec validate --strict` accepts once merged, so each is a requirement of its own. Systemd detection (`systemctl --user show-environment`, read-only) still runs before the refusal: install has to know a systemd unit is what it would write.

## Non-goals

- Changing the unit's, plist's or crontab line's content, or the 0644 mode of the unit and plist (spec'd).
- Taking over a foreign unit with a `--force` flag (the user moves it away).
- The runtime directory (already tightened, `daemon-lifecycle` "Runtime directory and socket modes").

## Impact

`crates/tilr/src/service.rs`, `crates/tilrd/src/identity.rs` (and its callers in `auth.rs`, `cloud.rs`, the supervisor start), `crates/tilrd/tests/auth.rs`, itests UC-020, README "Starting at boot". **BREAKING** (CLI behavior): `tilr service install` over a hand-written `tilr.service` or plist now fails instead of replacing it; migration: move the file away (or add the marker) and install again. No protocol change, no PROTOCOL_VERSION or feature change, nothing crosses a reload (the state directory is checked at start; a reload is not a start). An update from the previous release: the next start tightens the directory.
