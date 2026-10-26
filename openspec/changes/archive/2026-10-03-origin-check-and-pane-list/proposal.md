# Proposal: Origin check on the WebSocket handshake; PaneList broadcast and PaneInfo

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Without an origin check any web site open in the user's browser could open a shell on the server (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- Browser connections are accepted only from `--allow-origin` (default `http://127.0.0.1:7891`, `http://localhost:7891`); a client without an Origin header passes.
- The supervisor broadcasts `PaneList` to every client on any pane change (`request_id` 0 for a broadcast, the request's id for an explicit `ListPanes`).
- `PaneInfo` carries title, cwd (OSC 7), viewer count and creation time, fed by an internal pane-to-supervisor status packet.

## Capabilities

### Modified Capabilities
- `access-control`
- `panes`

## Out of scope

- Access token (added on 2026-10-09).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory notes `main/projects/tilr/tilr-zhurnal-reshenii`; the code landed in the initial commit [`264d1f4`](https://github.com/tilr-dev/tilrd/commit/264d1f4) Terminal server: pane processes behind a WebSocket (2026-10-04).
