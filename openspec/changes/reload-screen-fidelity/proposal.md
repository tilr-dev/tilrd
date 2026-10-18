# Proposal: Carry the remaining screen state across a pane reload

## Why

Known losses on a pane reload, recorded since 2026-10-03 and still open: the wrap flag at the history/screen border, a pending wrap, the `kill_at` timer, kitty placements whose top row is in the history, and non-text modes after a pane-process restart (alternate screen, mouse, hidden cursor) (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-kitty-grafika-v-terminale-plan-cherez-libghostty-vt-i-vydelenie-teksta-sdelano`).

## What Changes

- Carry soft-wrap at the border and pending wrap in the snapshot.
- Carry the `kill_at` timer of a pane being closed.
- Keep placements partly in the history.

## Capabilities

### Modified Capabilities
- `reload`: see `specs/reload/spec.md`.
- `screen-frames`: no requirement change.

## Out of scope

- Pane-process restarts after a crash (no state to carry).

## Impact

`crates/tilr-screen` snapshot/restore, pane state (new fields: bump the pane state format and `state-formats`). Must survive an update from the previous release: an old state without the fields restores as today.
