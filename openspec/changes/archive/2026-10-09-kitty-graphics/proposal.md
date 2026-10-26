# Proposal: Kitty graphics in frames, unicode placeholders, the client's cell size

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

The libghostty-vt crate feature for kitty graphics had been off since the first commit 264d1f4, so images shown by programs in panes never reached clients (Basic Memory `main/projects/tilr/tilr-kitty-grafika-v-terminale-plan-cherez-libghostty-vt-i-vydelenie-teksta-sdelano`).

## What Changes

- libghostty-vt built with `kitty-graphics` and `png`; a storage limit of 320 MB and a PNG decoder of our own (`RustPngDecoder` has no constructor in 0.2.2).
- Additive proto fields: `Frame.placements_changed`, `placements`, `images`; `Placement` with pixel size; images as PNG encoded once per (id, generation) and sent again on a full frame.
- `Resize.cell_width_px`/`cell_height_px` and the same in `Frame`: placements measured in the client's own cell.
- Unicode placeholders (U+10EEEE) become ordinary placements server-side.
- Snapshots re-emit images (`a=t,f=100`) and virtual placements (`U=1`), so a pane reload keeps them; a placement scrolled into the history is left out.

## Capabilities

### Modified Capabilities
- `screen-frames`
- `reload`

## Out of scope

- Placements whose top row is in the history across a reload (open change `reload-screen-fidelity`).

## Impact

Implemented by: [`befaf82`](https://github.com/tilr-dev/tilrd/commit/befaf82) Kitty graphics: placements and images in frames, kept across a pane reload (2026-10-09); [`9bed1a0`](https://github.com/tilr-dev/tilrd/commit/9bed1a0) Kitty graphics: the client's cell size, unicode placeholders, PNG in snapshots (2026-10-09). Clients: plugin-server 0.2.1, plugin-terminal 0.4.0. PROTOCOL_VERSION unchanged.
