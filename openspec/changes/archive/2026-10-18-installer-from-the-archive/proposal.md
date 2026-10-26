# Proposal: The installer installs from the release archive, all or nothing

> Retro archive entry, reconstructed on 2026-10-18 from git history (branch `dev`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

UC-004 was red: the installer made `~/.local/bin` before the download was checked and installed `tilrd` before `tilr` was verified (open change `installer-atomic-install`). The release archive of `2a8b425` changed what the installer fetches. Explicitness about starting at boot was already decided (UC-023: nothing starts tilrd at boot unless asked).

## What Changes

- `itests/installer/install.sh` rewritten: the archive is downloaded into `mktemp -d`, checked, unpacked, both programs run with `--version`; only then is `~/.local/bin` made and both renamed into place (`tilr` first, `tilrd` last, the old `tilr` put back on failure). Anything that fails installs nothing and leaves nothing in the home or `/tmp`.
- No systemd unit, no linger, no `TILR_NO_SERVICE`, no profile edits, no `tilr-server` migration: the installer ends with hints (`tilr`, `tilr service install`, `tilr token`, `tilr cloud login`) and, without `~/.local/bin` on `PATH`, the line for `~/.profile`.
- itests: `t.cdn().publish` serves the archive; UC-001..006 rewritten for it; UC-004 adds the archive cut short, the archive without `tilr`, a program that does not run and a `latest` without its archive, all green; UC-023 runs the installer with a systemd user manager present and checks it asks nothing; UC-06A and `uc_06a.py` removed; UC-060/065/066/067 publish archives and check the `tilr` next to `tilrd`.

## Capabilities

### Modified Capabilities
- `installation`
- `update`
- `tilr-cli`

## Out of scope

- The real installer in the `sh` repository and its publication to R2 (done there).

## Impact

Implemented by: [`3ba0e93`](https://github.com/tilr-dev/tilrd/commit/3ba0e93) test(itests): install from the release archive, fix installer UC-004 (2026-10-18). Settles the open change `installer-service-default` (archived); `installer-atomic-install` stays open for the port to the `sh` repository.
