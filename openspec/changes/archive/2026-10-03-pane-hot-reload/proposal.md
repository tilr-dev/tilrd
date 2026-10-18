# Proposal: Pane processes reload in place

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Updating the server must not close the user's shells. The earlier plan to hand descriptors over with SCM_RIGHTS was replaced by an exec in place, which keeps the pid, keeps the shell as a child and keeps the pipes open; a raw byte ring buffer was rejected because it loses the scrollback (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- On `kill -USR1 <supervisor>` the supervisor sends every pane an empty packet; the pane process execs the executable on disk with the same pid.
- The pty master and an unlinked state file are handed over (`pane --resume master:state:shell_pid:seq` at the time); screen and scrollback travel as a VT snapshot from libghostty's formatter (`Screen::snapshot` / `GhosttyScreen::restore`).
- If the exec fails the pane goes on with the old code.
- Known losses recorded then: the wrap flag at the history/screen border, a pending wrap, the `kill_at` timer, kitty graphics; the state file had no version yet.

## Capabilities

### Modified Capabilities
- `reload`

## Out of scope

- Reloading the supervisor itself (see `2026-10-04-supervisor-reload`).
- State format versions (see `2026-10-04-state-format-versions`).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory notes `main/projects/tilr/tilr-zhurnal-reshenii`; the code landed in the initial commit [`264d1f4`](https://github.com/tilr-dev/tilrd/commit/264d1f4) Terminal server: pane processes behind a WebSocket (2026-10-04).
