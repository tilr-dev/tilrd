# Proposal: Panes start in the home directory; nothing is sent before HelloAck

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A pane asked to start in no directory, or in one not on the server's machine, failed with a misleading spawn error (commit message).

## What Changes

- Such a pane starts in the home directory instead.
- Pane list broadcasts skip clients that have not said hello yet.

## Capabilities

### Modified Capabilities
- `panes`
- `protocol`

## Out of scope

- Nothing recorded.

## Impact

Implemented by: [`26449dd`](https://github.com/tilr-dev/tilrd/commit/26449dd) Panes start in the home directory; nothing is sent before HelloAck (2026-10-04).
