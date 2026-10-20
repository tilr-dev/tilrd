# Proposal: The commands the Tilr app runs are a contract of tilrd

## Why

- The Mac app (repository `tilr-dev/desktop-mac`) runs this repository's executable. What it runs, read on 2026-10-18 in its `tilrd-rename` checkout: `tilrd --version` (`ServerInstallation.swift:170`, prefix `tilrd <ver> (`), `tilrd ps --url <endpoint> [--server-version]` (`ServerInstallation.swift:381`, `LocalServer.swift:195,311`), `tilrd token` (`ServerInstallation.swift:483`), `tilrd state-formats` (`ServerInstallation.swift:216-217`), and a launchd job running `tilrd --listen 127.0.0.1:<port> --config <origins.toml>` with `TILR_STATE_DIR` and `TILR_RUNTIME_DIR` (`ServerInstallation.swift:175-181`); `app-managed.json` next to the executable makes `tilrd update` refuse (`crates/tilrd/src/update.rs:55` at 1163abd).
- `tilrd ps`, `tilrd token` and `tilrd state-formats` are hidden commands kept for the app (`crates/tilrd/src/main.rs:153-174` at 1163abd); `ps --server-version` and `state-formats` have no requirement saying so, so a change to their output would not be seen as breaking the app.

## What Changes

- A `daemon-lifecycle` requirement that the hidden commands the app runs (`ps --url ... --server-version`, `token`, `state-formats`) keep their output, so a change to them is seen as a change to the app's contract. No behavior change.
- A test for `ps --url ... --server-version` when nothing listens; the `--help` comments of the three commands point at the requirement.
- The app's own move to tilrd (desktop-mac `tilrd-rename`) and the release order are TILR-187's, not this change's (owner's decision, 2026-10-20).

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`: see `specs/daemon-lifecycle/spec.md`.

## Non-goals

- Metric names (`tilrd_*` and `tilr_*` are mixed in `crates/tilrd/src/metrics.rs:161-272` at 1163abd; the app does not read `/metrics`): left as they are.
- The desktop-mac, cloud, docs, sh and plugins repositories.
- A compatibility path for `tilr-server` names in this repository (decided against, 2a8b425).

## Impact

`openspec/specs/daemon-lifecycle` (one requirement), one test in `crates/tilrd/tests/panes.rs`, doc comments in `crates/tilrd/src/main.rs`. No behavior or protocol change; nothing crosses a reload.

## Commits

- `7cf8f06` docs(spec): keep the output of the hidden commands the Tilr app runs (requirement, test `panes::ps_server_version_fails_when_nothing_listens`, doc comments).
