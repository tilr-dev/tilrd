# Proposal: Restore the cursor correctly under origin mode after a pane reload

## Why

Found while implementing `reload-screen-fidelity` (TILR-74); the loss is older than that change. The pane snapshot (`crates/tilr-screen/src/ghostty.rs`, `snapshot`) carries the terminal modes (`SNAPSHOT_MODES` lists `Mode::ORIGIN`, DECOM, `?6`) and, through `format_range` with `Extras::All`, the scrolling region. The cursor goes back last, after the modes: `out.extend(modes)` ... `out.extend(cursor)`, where `cursor` is `cursor_position(&self.term, Extras::Pen)`, an absolute CUP built from `term.cursor_y()` / `cursor_x()` (`ghostty.rs:613`, `683`, `1289-1293`). Under DECOM a CUP is relative to the top margin of the scrolling region, so a program that set a region (`ESC [ top ; bottom r`) and origin mode and then stopped with the cursor on screen row `y` gets its cursor on row `y + top` after a reload (clamped to the bottom margin). Programs that use DECOM with a margin (some full-screen editors and pagers, status-line layouts) print into the wrong row until they reposition. With left/right margins (`?69`) the column has the same problem.

## What Changes

- The snapshot places the cursor so that it lands on the same cell whether or not origin mode is on: either the final CUP is translated by the margins when DECOM is on (design choice to confirm in the implementation: row minus the top margin, column minus the left margin), or the cursor is placed before DECOM is enabled and the modes are enabled without moving it (setting DECOM homes the cursor, so the order matters: this is why the translation is the likelier fix).
- The left and right margins themselves carry over: the snapshot turns `?69` on before the state's DECSLRM, which without the mode reads as a cursor save (SCOSC) and was lost (found by the unit test).
- A unit test and a reload test show the cursor on the same cell.

## Capabilities

### Modified Capabilities
- `reload`: one requirement added (the cursor under origin mode).

## Non-goals

- Other mode interactions in the snapshot (pending wrap is covered by `reload-screen-fidelity`).

## Impact

`crates/tilr-screen/src/ghostty.rs` (`snapshot`, `cursor_position`) and its unit tests, `crates/tilrd/tests/reload.rs`, USE_CASES.md, README "Reloading without closing shells". No wire, protocol or state format change (the snapshot stays a VT byte stream); size S.
