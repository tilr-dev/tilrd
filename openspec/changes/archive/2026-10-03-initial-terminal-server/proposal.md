# Proposal: First working terminal server

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A cell grid with diffs, not raw bytes, was chosen so that a client can reconnect, several clients can watch one pane and clients stay light (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- Workspace crates `tilr-proto` (protobuf schema `tilr.proto`, packet = flag byte + body, zstd level 1 for bodies of 256 B or more), `tilr-screen` (a `Screen` trait over libghostty-vt 0.2.2) and the server.
- Panes send cell-grid frames: rows, style runs, a full frame on subscribe and resize, diffs otherwise; the supervisor forwards pane frames without re-encoding.
- Input as key, mouse and paste events encoded by Ghostty's key encoder, plus raw bytes.
- Localhost only, no token; a test client in `~/projects/tilr/test-client`.
- Build notes: Zig 0.15.2 against the macOS 27 SDK needs an untracked `.cargo/config.toml` workaround; libghostty-vt-sys clones Ghostty at a pinned commit, so the first build needs network.

## Capabilities

### Modified Capabilities
- `protocol`
- `panes`
- `screen-frames`

## Out of scope

- Authentication (localhost only at the time; see `2026-10-09-access-token-and-config-file`).
- Kitty graphics (crate feature off until `2026-10-09-kitty-graphics`).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory notes `main/projects/tilr/tilr-zhurnal-reshenii`; the code landed in the initial commit [`264d1f4`](https://github.com/tilr-dev/tilrd/commit/264d1f4) Terminal server: pane processes behind a WebSocket (2026-10-04).
