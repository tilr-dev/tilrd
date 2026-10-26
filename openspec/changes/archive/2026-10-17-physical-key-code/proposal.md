# Proposal: KeyInput.code: Ctrl and Alt by the physical key

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-079 was red: Ctrl+C on a Russian layout gave nothing (itests/USE_CASES.md).

## What Changes

- `KeyInput.code` (field 7) carries the physical key, as W3C `KeyboardEvent.code`.
- With Ctrl or Alt and a non-ASCII character the server encodes the key's US character; an ASCII character wins (Dvorak, AZERTY).
- Mac and iPad clients must send `code` (client-side follow-up).

## Capabilities

### Modified Capabilities
- `screen-frames`

## Out of scope

- Option+c, AltGr and other non-Latin Ctrl/Alt cases (open change `non-latin-ctrl-alt`).

## Impact

Implemented by: [`c4444cf`](https://github.com/tilr-dev/tilrd/commit/c4444cf) Screens ahead of transfers, bounded streams, and Ctrl on any layout … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
