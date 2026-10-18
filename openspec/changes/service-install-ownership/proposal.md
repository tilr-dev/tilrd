# Proposal: tilr service install leaves foreign units alone; state directory made private

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (findings #23, #24): `tilr service install` overwrites a `tilr.service` it did not write (no marker check) and `uninstall` removes it (`service.rs:147-151,291`); crontab detection matches English messages only; `write_private` writes 0644; an existing state directory is not tightened to 0700 (`identity.rs` `create_private_dir`).

## What Changes

- Install refuses to replace a unit, plist or crontab line without its marker and says how to proceed; uninstall removes only marked ones.
- Crontab detection by exit status, not by message text (`LC_ALL=C` for the calls).
- Files are written with the mode their name says (rename `write_private` or make it 0600 where the content is private).
- An existing state directory is set to 0700 at start.

## Capabilities

### Modified Capabilities
- `tilr-cli`: see `specs/tilr-cli/spec.md`.
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md`.

## Out of scope

- Changing the unit's content.

## Impact

`crates/tilr/src/service.rs`, `crates/tilrd/src/identity.rs`. Nothing crosses a reload; no protocol change.
