# Proposal: Carry the remaining screen state and the close deadline across a pane reload

## Why

Known losses on a pane reload, recorded since 2026-10-03 and still open (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-kitty-grafika-v-terminale-plan-cherez-libghostty-vt-i-vydelenie-teksta-sdelano`): the wrap flag at the history/screen border, a pending wrap, the `kill_at` timer, kitty placements whose top row is in the history, and non-text modes. Re-checked at 1163abd:
- Border wrap: the snapshot formats the history, then pushes it off with a reset and `\r\n` per screen row, then draws the screen from `ESC [H` (`crates/tilr-screen/src/ghostty.rs:625-646`), so a line soft-wrapped from the last history row into the first screen row comes back as two lines.
- Pending wrap: the cursor is restored with a plain CUP (`cursor_position`, `ghostty.rs:1130-1133`; used at `651` and `661`), so a cursor waiting to wrap at the last column (after exactly `cols` characters) loses that state and the next character overwrites the last cell.
- `kill_at`: `ClosePane` arms a 2 s deadline (`crates/tilrd/src/pane.rs:603-606`, `390-393`) that is not in `PaneHandover` (`pane.rs:519-535`; `crates/tilr-proto/proto/tilr.proto:1059-1081`); a successor starts with `kill_at: None` (`pane.rs:329`), so a program ignoring SIGHUP after a close survives a reload in that window.
- Images: `kitty_images` skips a placement whose viewport row is negative (`ghostty.rs:1050-1056`); the unit test `snapshot_leaves_out_an_image_scrolled_into_the_history` (`ghostty.rs:2148-2166`) records that loss.
- Non-text modes are already carried (`SNAPSHOT_MODES`, `ghostty.rs:664-704`; the `reload` requirement "Screens, scrollback and programs carry over", UC-061): no work left there.

## What Changes

- The snapshot keeps a soft wrap between the last history row and the first screen row (design D1).
- The snapshot restores a pending wrap at the last column (design D2).
- `PaneHandover` carries the close deadline: the milliseconds left and the process groups being ended (design D3); the successor arms it, so the program is still killed when the deadline passes.
- Kitty placements whose top row is in the history are placed again while the history is replayed (design D4); the test that records the loss is turned around.

## Capabilities

### Modified Capabilities
- `reload`: three requirements added (soft wraps, close deadline, images in the history).

## Non-goals

- Pane-process restarts after a crash (no state to carry).
- Blank history rows the libghostty formatter drops (the comment in `snapshot_leaves_out_an_image_scrolled_into_the_history`); that is formatter behavior (change `libghostty-upstream-fixes`).
- The scroll position (already carried, `scrolled_back`).

## Impact

`crates/tilr-screen/src/ghostty.rs` (snapshot) and its unit tests, `crates/tilrd/src/pane.rs` (handover, deadline), `crates/tilr-proto/proto/tilr.proto` (`PaneHandover`: two fields; internal reload state, not a wire message: no PROTOCOL_VERSION or feature change; it lands after `protocol-v3` only to keep proto edits in order), `crates/tilrd/tests/reload.rs`, USE_CASES.md, README "Reloading without closing shells". Survives a reload by definition. No pane state format bump (design D5): the new fields are additive with "none" as the zero value and the snapshot stays a VT byte stream any reader replays, so `tilrd state-formats` is unchanged; an update from the previous release reloads into this one with today's behavior for that first reload (no deadline handed over), and a rollback ignores the new fields. Not breaking.

Implemented by: [`53e7ff2`](https://github.com/tilr-dev/tilrd/commit/53e7ff2) fix(reload): carry wraps, images in the history and the close deadline (2026-10-18).
