# Proposal: App-managed macOS server releases; version health check

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The Mac app bundles and updates its own server through a signed feed, separately from its Sparkle updates, and needs to ask a running server for its version (commit diff: README "macOS application distribution").

## What Changes

- Hidden `ps --server-version` prints the connected server's version (a health check for the app).
- `update` refuses when `app-managed.json` lies next to the executable ("this server is managed by Tilr.app; use Tilr > Check for Server Updates …"); README documents the app-managed lifecycle.
- `TILR_BUILD_VERSION` overrides the package version in the version stamp (build.rs); tests in `tests/panes.rs` and `tests/update.rs`.

## Capabilities

### Modified Capabilities
- `update`
- `panes`

## Out of scope

- The app's feed and LaunchAgent (tilr-mac repository).

## Impact

Implemented by: [`f20572c`](https://github.com/tilr-dev/tilrd/commit/f20572c) Support app-managed macOS server releases and version health checks (2026-10-11). Rationale beyond the README text not recorded.
