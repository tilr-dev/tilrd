# Proposal: The installer sets up no service

## Why

The installer at 13f9522 wrote and started its own `tilr.service` and turned on linger wherever a systemd user session existed (unless `TILR_NO_SERVICE=1`), while the `tilr-cli` spec (UC-023) says nothing starts `tilrd` at boot unless the user asks. The decision recorded in Basic Memory (`main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`, `main/projects/tilr/tilrd-idei-read-only-zritel-client-id`) is "service not installed by default (hint `tilr service install`)". This change was open with that question for the owner; `3ba0e93` implemented option (a), the recorded decision, so the question is settled.

## What Changes

- The installer writes no unit, calls no `systemctl`, `loginctl`, `crontab` or `sudo`, and ends with the hint `start tilrd at boot: tilr service install` (among `tilr`, `tilr token`, `tilr cloud login`). `TILR_NO_SERVICE` is gone.
- Starting at boot is only `tilr service install` (one unit format, `Type=notify`, its own marker).
- UC-023 runs the installer where a systemd user manager answers and checks it asked nothing; UC-022's "unit it did not write" is a hand-written unit.

## Capabilities

### Modified Capabilities
- `installation`
- `tilr-cli`

## Out of scope

- A `tilr.service` written by an earlier installer is not migrated: `tilr service uninstall` leaves it (no marker). Only pre-release installs have one; no change is open for it.
- The port to the `sh` repository's installer (open change `installer-atomic-install`).

## Impact

Implemented by: [`3ba0e93`](https://github.com/tilr-dev/tilrd/commit/3ba0e93) test(itests): install from the release archive, fix installer UC-004 (2026-10-18), in `itests/installer/install.sh`; README "Install" and "Starting at boot" (merged in 47a8557) say the same. Nothing crosses a reload.
