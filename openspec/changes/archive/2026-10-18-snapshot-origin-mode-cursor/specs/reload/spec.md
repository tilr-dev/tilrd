## ADDED Requirements

### Requirement: The cursor is where it was under origin mode
The cursor of a pane with a scrolling region, or left and right margins (`?69`), and origin mode on SHALL be on the same cell after a pane reload as before it, a pending wrap included; the left and right margins SHALL be the same too.

**Verified by**: unit test `ghostty::tests::snapshot_keeps_the_cursor_under_origin_mode`; `reload::the_cursor_under_origin_mode_is_where_it_was_after_a_reload`.

#### Scenario: Region and origin mode
- **WHEN** a program sets the scrolling region to rows 3 to 8, turns on origin mode, moves the cursor to the second row of the region, and the server reloads
- **THEN** the cursor is on the second row of the region, and the next character printed lands there

#### Scenario: Left and right margins
- **WHEN** a program turns on `?69`, sets the left and right margins to columns 5 to 15 and the scrolling region, turns on origin mode, and the server reloads
- **THEN** the cursor is on the same cell, and text printed from there wraps at column 15 to column 5 of the next row
