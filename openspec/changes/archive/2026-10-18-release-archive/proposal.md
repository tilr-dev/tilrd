# Proposal: One release archive holds tilrd and tilr

> Retro archive entry, reconstructed on 2026-10-18 from git history (branch `dev`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

From the commit: the server and its client are always installed and updated together, so they cannot drift. The owner decided on 2026-10-18 that there is no compatibility for updaters from before the archive: they reinstall once.

## What Changes

- A release is one archive per platform, `releases/<version>/tilr-<version>-<platform>.tar.gz` (`tilrd`, `tilr`, licenses, README) with its `.sha256`, instead of the loose `tilrd-`, `tilr-server-` and `tilr-<platform>` files. **BREAKING**: a server from before (`tilr-server`, or a `tilrd` asking for the loose file) cannot update itself and is reinstalled once.
- `tilrd update` downloads the archive, checks its checksum, unpacks it into a private `.tilr-update.<pid>` next to the executable, checks both programs run and renames `tilr` first, `tilrd` last; if `tilrd` cannot be put in place the old `tilr` is put back (or the new one removed). Leftover staging directories are swept under the update lock. The old-name fallback is removed.
- `scripts/release.sh` packs and publishes the archive, `linux-build.sh` writes `dist/<platform>/{tilrd,tilr}`, `update-test.sh` installs from the archive; README describes the archive, the installer and manual installation.
- Tests: `update::a_server_that_cannot_be_put_in_place_puts_the_old_client_back` (failpoint `update-between-renames`); the archive cases of `update::update_installs_nothing_that_fails_its_checksum_or_does_not_run`; `update::a_release_from_before_the_rename_is_found_under_its_old_name` removed.

## Capabilities

### Modified Capabilities
- `update`
- `release-process`
- `installation`

## Out of scope

- Signing releases (`scoped-cdn-token`, `release-platforms` stay open).
- The Mac app's own server feed (`tilr-mac-rename-compatibility`).

## Impact

Implemented by: [`2a8b425`](https://github.com/tilr-dev/tilrd/commit/2a8b425) feat(update)!: ship tilrd and tilr as one release archive (2026-10-18). Survives a reload unchanged (the reload is what installs it); an update from a release before it is not possible (breaking, by decision).
