## ADDED Requirements

### Requirement: Clear a pane's screen and optionally its history
`ClearPane{pane_id, history}` SHALL erase the active screen and home the cursor, retaining scrollback unless `history` is true. It SHALL return the shared view to the live screen and send a full frame to every viewer, even if already empty. Clearing SHALL NOT write to the pty or stop the program. Subsequent subscribers SHALL see the cleared screen. An unknown pane SHALL receive an error without disconnecting.

**Verified by**: integration tests `clear_pane::clearing_history_reaches_all_viewers_and_survives_reconnecting`, `clear_pane::clearing_only_the_screen_keeps_history`, `clear_pane::clearing_does_not_write_to_the_program_and_output_starts_at_home` `clear_pane::clearing_an_unknown_pane_is_an_error` and unit test `ghostty::tests::erasing_homes_the_cursor_with_origin_mode_and_keeps_the_mode`.

#### Scenario: Clear screen and history while scrolled back
- **WHEN** a pane has scrollback, two viewers watch it scrolled back, and one sends `ClearPane{history: true}`
- **THEN** both receive a full blank frame at the live screen, scrolling up reveals no history, and reconnecting shows a blank screen

#### Scenario: Keep history
- **WHEN** a pane with scrollback receives `ClearPane{history: false}`
- **THEN** its live screen is blank and its cursor is at (0, 0), and scrolling up still shows the retained history

#### Scenario: Continue the program
- **WHEN** a program waiting for input has its pane cleared
- **THEN** it receives no bytes from clearing, remains running, and its next output starts at the first cell, even with origin mode and a scrolling region enabled; those modes remain unchanged

#### Scenario: Clear an already empty pane
- **WHEN** a viewer clears an already empty pane
- **THEN** every viewer receives a full blank frame

#### Scenario: Unknown pane
- **WHEN** a client sends `ClearPane` for a nonexistent pane
- **THEN** it receives `Error{message: "no such pane: <pane_id>"}` and remains connected
