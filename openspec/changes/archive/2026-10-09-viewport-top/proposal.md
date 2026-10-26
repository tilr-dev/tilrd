# Proposal: Frame.viewport_top: selection follows the viewport

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A client that keeps a selection at screen rows lost it when output scrolled; it needs to know where the viewport is in the history (Basic Memory `main/projects/tilr/tilr-avtoskroll-pri-vydelenii-i-frame.viewport-top-vydelenie-sleduet-za-viewport-sdelano`).

## What Changes

- `Frame.viewport_top` (field 17): history rows above the viewport, from Ghostty's scrollbar offset; fixed while scrolled back, growing by one per line at the live bottom; a change alone makes a frame. Additive.

## Capabilities

### Modified Capabilities
- `screen-frames`

## Out of scope

- Selection shifting when the scrollback is trimmed (known caveat).

## Impact

Implemented by: [`62e640c`](https://github.com/tilr-dev/tilrd/commit/62e640c) Frame.viewport_top: where the viewport is in the history, so clients can follow it (2026-10-09).
