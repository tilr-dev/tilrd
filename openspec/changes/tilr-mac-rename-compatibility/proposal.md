# Proposal: Server-side steps for the tilr-mac rename to tilrd

## Why

Merging the itests branch (rename `tilr-server` to `tilrd`) breaks tilr-mac: `bundle_server.py` (`-p tilr-server`, `release/tilr-server`), `ServerInstallation.swift` waits for the output `tilr-server <ver> (`, `release_server.py`, `verify_release.py`, `ServerUpdates.swift`, tests and docs; also `install.sh` and Cloud texts. A tilrd release must be published with `--min-app-build` = the first Mac app build that supports tilrd; the old CDN feed with component `tilr-server` is to be deleted or recreated on first publish. A daemon started by `tilr` may take port 7890 before the Mac app, so the app must attach to a running tilrd (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`, `main/projects/tilr/tilr-cli-tilr-iz-tilr-dev-plugin-razbor-i-resheniia`). The transition (two artifact names, the old-name fallback) is gone since `2a8b425`: a release is one archive per platform, `tilr-<version>-<platform>.tar.gz` holding `tilrd` and `tilr`, and the owner decided on 2026-10-18 there is no compatibility for old updaters (they reinstall once).

## What Changes

- Agree the merge order (web `drop-tilr-cli` → plugins; tilrd `itests` → `main` together with tilr-mac `tilrd-rename` and `--min-app-build`; cloud, docs and sh after the tilrd release).
- Server side: keep `tilrd --version` output stable (`tilrd <ver> (<platform>)`) and `ps --server-version`. No two-name transition (decided, removed in 2a8b425).
- The Mac app handles the new release form: the GitHub release assets are `tilr-vX.Y.Z-<target>.tar.gz` (directory `tilr-vX.Y.Z-<target>/` with `tilrd`, `tilr`, licenses, README) and the CDN has `tilr-<version>-<platform>.tar.gz` for Linux only; `bundle_server.py` / `release_server.py` / `verify_release.py` take `tilrd` from the archive (or build `-p tilrd`), and the app's own signed server feed names `tilrd`. An app-managed copy still refuses `tilrd update` (`app-managed.json`).

## Capabilities

### Modified Capabilities
- `release-process`
- `update`

No spec delta: tests, docs or process only; `skip_specs: true`.

## Out of scope

- The tilr-mac, cloud, docs and sh repository changes (prepared on their `*-tilrd-rename` branches).

## Impact

Release process and the Mac app's server feed; no protocol change. **BREAKING**: an installation from before the archive (a `tilr-server`, or a `tilrd` asking for loose files) does not update itself and is reinstalled once; the Mac app's managed copy moves through the app's feed, not the CDN layout.
