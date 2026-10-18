# Proposal: Name, license and distribution decisions

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The project needed a name, a license and a way to reach users' machines (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- Name Tilr (from "tiller"); package names `tilr-cli` / `tilrd`; GitHub organization `tilr-dev`.
- License MIT OR Apache-2.0.
- Distribution by a curl installer (cargo-dist planned) and built-in self-update from GitHub Releases (`self_update` crate); later superseded by the CDN layout of `2026-10-04-update-from-cdn`.

## Capabilities

### Modified Capabilities
- `installation`
- `update`
- `release-process`

## Out of scope

- Implementation (installer and update came later and differently).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory notes `main/projects/tilr/tilr-zhurnal-reshenii`; the code landed in the initial commit [`264d1f4`](https://github.com/tilr-dev/tilrd/commit/264d1f4) Terminal server: pane processes behind a WebSocket (2026-10-04). This entry records decisions; the license files arrived with `2026-10-16-tilr-dev-repo-standard`.
