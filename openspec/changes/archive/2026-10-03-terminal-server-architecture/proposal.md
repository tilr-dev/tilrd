# Proposal: Server-side virtual terminals over WebSocket (architecture)

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The goal (then called RIG) was a service on a remote host that holds virtual terminals, runs software in them and serves them to a browser terminal. Herdr was studied as a reference and gave only two relevant pieces: a portable pty and a VT library. One process per pane under a supervisor was chosen for isolation (a hung pane restarts alone), and the pty had to live on the server side because a pty slave cannot be swapped under a running process (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- One supervisor process with a WebSocket front; one child process per pane owning its pty, shell and VT parser.
- The supervisor routes packets between clients and pane processes and does not interpret screens.
- Recorded as the architecture of Tilr "core" (later `tilr-server`, then `tilrd`).

## Capabilities

### Modified Capabilities
- `protocol`
- `panes`

## Out of scope

- Code (see `2026-10-03-initial-terminal-server`).
- Remote desktop / virtual display (recorded as a direction only).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory notes `main/projects/tilr/tilr-zhurnal-reshenii`; the code landed in the initial commit [`264d1f4`](https://github.com/tilr-dev/tilrd/commit/264d1f4) Terminal server: pane processes behind a WebSocket (2026-10-04). This entry is the design; it has no code of its own.
