# Design

## Context

Frames are built from libghostty's render state: for each dirty row, cells are read through the row's cell iterator and merged into runs by style (`crates/tilr-screen/src/ghostty.rs:302-365`, `RunBuilder`). Styles are deduplicated per frame into `Frame.styles` (`StyleTable`). libghostty-vt 0.2.2: `Row::has_hyperlink()` (may give false positives), `Cell::has_hyperlink()`, and `Terminal::grid_ref(Point)` + `GridRef::hyperlink_uri(buf)` for the URI. The render state has no URI accessor.

## Decisions

### D1. A per-frame link table, like styles
`Frame.links` lists each URI used by the frame's `changed` rows once; `Run.link` is its index plus one. A frame without links has an empty list and every `Run.link` 0, so it encodes to the same bytes as today. Rejected: the URI in every `Run` (repeats long URIs on every row of a listing); a link table that lives across frames (would need resending on full frames and after a skipped frame for a lagging client, as images do; a per-frame table has no state to resync).

### D2. Reading URIs only where there are links
For a dirty row, `Row::has_hyperlink()` is asked first; only then is each cell's `Cell::has_hyperlink()` checked, and for a cell that has one the URI is read with `grid_ref(Point::Viewport{x, y})` from the terminal, which the render state was just updated from, so the positions agree. A URI is read once per run (a run continues while the next cell has the same URI). Rejected: reading every cell's URI (a grid lookup per cell on every frame).

### D3. Limits
A URI over 8 KiB is not put in the table and its cells go without a link; the frame is otherwise unchanged. The number of links per frame is bounded by the cells of the frame. URIs are passed as the program gave them (any scheme); the client decides what a click does.

### D4. No flag, no version bump
Per the protocol version policy in `openspec/config.yaml`, version 1 remains unchanged before the first public release. The fields have a harmless default, and no server can be run without them, so they need no feature flag. A v1 client that ignores them shows plain text.

### D5. tilr attach
`draw` (`crates/tilr/src/attach.rs:318`) writes `ESC ] 8 ; ; <uri> ESC \` before a run with a link and `ESC ] 8 ; ; ESC \` after it, so the local terminal's own link handling applies.

## Reload and reconnect

A reload hands the screen over as a VT snapshot, which has no per-cell links (Ghostty's formatter emits them for HTML only); after the reload those cells are plain text, and new output carries links again. This is recorded as a limit in README and the spec, and the fix (OSC 8 per cell in the VT formatter) goes to `libghostty-upstream-fixes`. No new state crosses the reload and no state format changes. A client that reconnects gets a full frame whose table lists the links in view.

## Backpressure

Links ride in frames on the interactive lane; a frame's size grows by its URIs at most once each (D1), bounded by D3. A lagging client skips frames as today and gets a full frame with its own table.
