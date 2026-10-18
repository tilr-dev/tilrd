# Proposal: The Tilr app moves to tilrd (tracking change; the work is in desktop-mac)

Зависит от: protocol-v3 (the app's `server` plugin must speak the version the first release speaks)

## Why

- Merging the itests branch (rename `tilr-server` → `tilrd`, the release archive) breaks the Mac app (repository `tilr-dev/desktop-mac`): `bundle_server.py` (`-p tilr-server`, `release/tilr-server`), `ServerInstallation.swift` waiting for `tilr-server <ver> (`, `release_server.py`, `verify_release.py`, `ServerUpdates.swift`, tests and docs. A tilrd release must be published to the app's feed with `--min-app-build` = the first app build that supports tilrd; the old feed with component `tilr-server` is to be deleted or recreated on first publish. A daemon started by `tilr` may take port 7890 before the app, so the app must attach to a running tilrd (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`, `main/projects/tilr/tilr-cli-tilr-iz-tilr-dev-plugin-razbor-i-resheniia`).
- On 2026-10-18 the rename is prepared, uncommitted, in `~/projects/tilr/tilr-mac-tilrd-rename` (branch `tilrd-rename`, 19 files: `Sources/Tilr/ServerInstallation.swift`, `ServerUpdates.swift`, `scripts/bundle_server.py`, `release_server.py`, `verify_release.py`, tests, docs); `tilr-dev/desktop-mac` on GitHub has only `main`.
- What the app runs in this repository's executable, read in that checkout: `tilrd --version` (`ServerInstallation.swift:170`, prefix `tilrd <ver> (`), `tilrd ps --url <endpoint> [--server-version]` (`ServerInstallation.swift:381`, `LocalServer.swift:195,311`), `tilrd token` (`ServerInstallation.swift:483`), `tilrd state-formats` (`ServerInstallation.swift:216-217`), and a launchd job running `tilrd --listen 127.0.0.1:<port> --config <origins.toml>` with `TILR_STATE_DIR` and `TILR_RUNTIME_DIR` (`ServerInstallation.swift:175-181`); `app-managed.json` next to the executable makes `tilrd update` refuse (`crates/tilrd/src/update.rs:55` at 1163abd).
- The app refuses an update unless the old and new `state-formats` output is identical (`ServerInstallation.swift:218`). Since `4298b15` (reload-rollback) `state-formats` has a `reload-check 1` line and the server itself refuses a successor it cannot resume into, so the app's own equality check refuses every update across that line and duplicates the server's check.
- Since `2a8b425` a release is one archive per platform; the owner decided on 2026-10-18 there is no compatibility for old updaters (they reinstall once). `tilrd ps`, `tilrd token` and `tilrd state-formats` are hidden commands kept for the app (`crates/tilrd/src/main.rs:153-174` at 1163abd); `ps --server-version` and `state-formats` have no requirement saying so.

## What Changes

- This repository: a `daemon-lifecycle` requirement that the hidden commands the app runs (`ps --url ... --server-version`, `token`, `state-formats`) keep their output, so a change to them is seen as a change to the app's contract. No behavior change.
- desktop-mac (`tilrd-rename`, tracked here): take `tilrd` from `tilr-vX.Y.Z-<target>.tar.gz` of the GitHub release (or build `-p tilrd`); expect `tilrd <ver> (`; drop the app's `state-formats` equality check and let the server's reload check decide, telling a refused reload by `ps --server-version` still naming the old version afterwards (as `LocalServer.swift:311` already reads); attach to a tilrd already serving the port instead of failing; speak protocol v3 in the `server` plugin.
- Release order (recorded above): web `drop-tilr-cli` → plugins; tilrd `dev` → `main` together with desktop-mac `tilrd-rename` and its feed published with `--min-app-build`; cloud, docs and sh after the tilrd release. The app's feed for component `tilr-server` is recreated as `tilrd` on first publish.
- **BREAKING** (already landed in `2a8b425`, recorded here): an installation from before the archive does not update itself and is reinstalled once; the app's managed copy moves through the app's own feed.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md`.

## Non-goals

- Metric names (`tilrd_*` and `tilr_*` are mixed in `crates/tilrd/src/metrics.rs:161-272` at 1163abd; the app does not read `/metrics`): left as they are.
- The cloud, docs, sh and plugins repositories' rename branches.
- A compatibility path for `tilr-server` names in this repository (decided against, 2a8b425).

## Impact

This repository: `openspec/specs/daemon-lifecycle` only (one requirement, existing tests). desktop-mac: `Sources/Tilr/{ServerInstallation,ServerUpdates,LocalServer}.swift`, `scripts/{bundle_server,release_server,verify_release}.py` and their tests. Nothing crosses a reload; no protocol change here (the app follows protocol-v3). An app update from a server before `4298b15` reloads only once the app drops its equality check; until then it says "needs a session migration" and leaves the terminals running.
