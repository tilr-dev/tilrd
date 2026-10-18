## ADDED Requirements

### Requirement: The cursor is where it was under origin mode
The cursor of a pane with a scrolling region and origin mode on SHALL be on the same cell after a pane reload as before it.

**Verified by**: unit test `ghostty::tests::snapshot_keeps_the_cursor_under_origin_mode` and `reload::the_cursor_under_origin_mode_is_where_it_was_after_a_reload` (planned in change snapshot-origin-mode-cursor).

#### Scenario: Region and origin mode
- **WHEN** a program sets the scrolling region to rows 3 to 8, turns on origin mode, moves the cursor to the second row of the region, and the server reloads
- **THEN** the cursor is on the second row of the region, and the next character printed lands there
