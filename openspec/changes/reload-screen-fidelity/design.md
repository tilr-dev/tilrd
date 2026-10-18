# Design

## Context

The pane snapshot (`GhosttyScreen::snapshot`, `crates/tilr-screen/src/ghostty.rs:566-662`) is VT bytes that rebuild the screen when fed to a fresh terminal: history formatted and pushed up, the active screen drawn from the home position, kitty images, cursor, terminal state, modes, title. `PaneHandover` (`tilr.proto:1059-1081`) carries it with the pty, the shell pid and the input bookkeeping. The reload check child (archived `reload-rollback`) restores the same snapshot from a copy of the state to show the successor can resume.

## Decisions

### D1. Border wrap
When the last history row is soft-wrapped into the first screen row, the snapshot formats that pair as one range (history row + screen row 0), so the formatter emits them as one wrapped line, and pushes the rest of the history off before it as today. The push (`\r\n` per row) is then counted so the joined line lands with its second part on screen row 0. Rejected: formatting history and screen as one range always (the formatter's blank-row handling would move the screen).

### D2. Pending wrap
If the cursor has a pending wrap (libghostty's cursor state), the snapshot ends by moving to the last column of the cursor row and re-printing that cell (its character and style, as the formatter gives it for that one cell), which sets the pending wrap exactly as the program left it; the style is then restored from the saved state. This is what tmux does on redraw. If libghostty-vt does not expose the pending-wrap flag, the apply step adds the accessor in the vendored binding or records it in `libghostty-upstream-fixes` and leaves D2 out (the change's other parts stand alone).

### D3. Close deadline in the handover
`PaneHandover` gets `uint32 kill_in_ms` (0 = no deadline) and `repeated uint32 kill_pgids` (the shell's and the foreground group being ended, as `sighup-ignoring-shell-reaped` defines). The old process writes the time left (at least 1 ms when a deadline is armed); the successor arms `kill_at = now + kill_in_ms` for those groups. The check child does not arm anything (it runs no loop).

### D4. Images in the history
While replaying the history, the snapshot splits the history range at each row where a placement's top row is, places the image at the cursor there (kitty `a=p` with `C=1`, so the cursor does not move) and carries on. Images are transmitted once, as today.

### D5. No state format bump
Proto3 fields with zero meaning "none" are read as "no deadline" by a reader of format 3 written before this change, and ignored by one that does not know them; the snapshot is replayed, not parsed. Bumping the format would refuse reloads between releases for nothing: under the reload check a successor that cannot read the state is refused anyway.

## Reload and reconnect

This change is about the reload itself: everything above is produced by the old pane process before its exec and consumed by the successor (and by the check child, which only restores the screen). Clients reconnect after a reload as today and get a full frame from the restored screen, now with the joined line, the pending wrap and the images in the history.

## Backpressure

Not applicable: no client data; the snapshot is a file in the runtime directory as before (images in the history make it larger, bounded by libghostty's image storage limit).
