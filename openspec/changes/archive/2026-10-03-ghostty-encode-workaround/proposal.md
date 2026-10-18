# Proposal: Work around libghostty-vt encode_to_vec; repaint a restarted pane

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A pane process crashed on mouse motion because libghostty-vt's `encode_to_vec` returned OutOfSpace (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, 2026-10-03).

## What Changes

- Encoding goes through `encode_into` with a buffer of our own in `tilr-screen` (ghostty.rs).
- A restarted pane resizes its pty to 2x2 for a moment so the program repaints.

## Capabilities

### Modified Capabilities
- `screen-frames`

## Out of scope

- Reporting the bug upstream (suggested to uzaaft/libghostty-rs, not recorded as done; see the open change `libghostty-upstream-fixes`).

## Impact

Implemented by: Pre-repository: reconstructed from Basic Memory notes `main/projects/tilr/tilr-zhurnal-reshenii`; the code landed in the initial commit [`264d1f4`](https://github.com/tilr-dev/tilrd/commit/264d1f4) Terminal server: pane processes behind a WebSocket (2026-10-04).
