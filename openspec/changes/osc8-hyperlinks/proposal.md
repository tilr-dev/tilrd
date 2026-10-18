# Proposal: OSC 8 hyperlinks in frames

## Why

Hyperlinks (OSC 8) are listed as not done (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`); `tilr.proto` has no hyperlink field at 13f9522.

Checked at 1163abd: still not done. `Run` has `style`, `text`, `wide`, `cluster` only (`crates/tilr-proto/proto/tilr.proto`, message `Run`); rows are built from the render state's cells without looking at links (`crates/tilr-screen/src/ghostty.rs:302-365`). libghostty-vt 0.2.2 keeps OSC 8 links per cell (`Cell::has_hyperlink`, `GridRef::hyperlink_uri`, `Row::has_hyperlink`), so the screen model already has them. Its VT formatter, used for the reload snapshot (`ghostty.rs:1139-1168`), emits only the cursor's current link, not the links of cells (Ghostty `src/terminal/formatter.zig`: "We currently only emit hyperlinks for HTML"), so links already on the screen do not survive a reload today either.

## What Changes

- `Frame` gets `repeated string links` (the URIs the frame's rows use) and `Run` gets `uint32 link` (index into `Frame.links` plus one, 0 for none); runs split where the link changes, as they do where the style changes.
- The pane process reads a cell's URI only in rows that have links, so screens without links cost nothing more.
- A URI longer than 8 KiB is not sent (its cells go as plain text).
- `tilr attach` shows the links in the local terminal (OSC 8 around the run), so terminals that support them make them clickable.
- No feature flag and no PROTOCOL_VERSION bump (protocol-v3 rule): new fields a client may ignore, which a server cannot be run without; a client that ignores them shows the text without links.
- Across a reload, links on the screen and in history become plain text (the snapshot has no per-cell links, see Why); output after the reload has links again. Recorded as a known limit; a fix belongs upstream (libghostty's VT formatter emitting OSC 8 per cell).

## Capabilities

### Modified Capabilities
- `screen-frames`: see `specs/screen-frames/spec.md` (two requirements added).

## Non-goals

- Opening links: what a client does with a click (and which schemes it opens without asking) is the client's choice.
- The OSC 8 `id=` parameter: libghostty-vt exposes only the URI; cells with the same URI next to each other are one link.
- Keeping links across a reload (needs the upstream formatter change; listed for change `libghostty-upstream-fixes`).
- Detecting plain-text URLs that have no OSC 8 (client side).

## Impact

`crates/tilr-proto/proto/tilr.proto` (`Frame.links`, `Run.link`; additive), `crates/tilr-screen/src/ghostty.rs` (run building), `crates/tilr/src/attach.rs` (draw), README "Protocol", USE_CASES.md. The `server` plugin and tilr-mac copy the schema and may render links later. Reload: nothing new crosses it; no state format change (links in the snapshot are lost as today). Update from the previous release: works; clients that do not know the fields ignore them.

Not breaking: additive messages and fields only, no PROTOCOL_VERSION bump, no close code, state file or CDN change.
