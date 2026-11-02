## ADDED Requirements

### Requirement: Clear a pane's screen and optionally its history
`ClearPane{pane_id, history}` SHALL erase the pane's active screen and move its cursor to the first cell, like `ESC[2J` followed by cursor home. With `history: true` it SHALL also erase scrollback, like `ESC[3J`; with `history: false` it SHALL retain scrollback. It SHALL reset the shared view to the live screen and send a full frame to every viewer, including when the screen is already empty. Clearing SHALL operate on the emulator without writing to the pty or stopping the program. A subsequent `Subscribe` SHALL show the cleared screen. An unknown pane SHALL receive `Error` with message `no such pane: <pane_id>`, without disconnecting the client.

**Verified by**: integration tests `clear_pane::clearing_history_reaches_all_viewers_and_survives_reconnecting`, `clear_pane::clearing_only_the_screen_keeps_history`, `clear_pane::clearing_does_not_write_to_the_program_and_output_starts_at_home` and `clear_pane::clearing_an_unknown_pane_is_an_error`.

#### Scenario: Clear screen and history while scrolled back
- **WHEN** a pane has scrollback, two viewers watch it scrolled back, and one sends `ClearPane{history: true}`
- **THEN** both receive a full blank frame at the live screen, scrolling up reveals no history, and reconnecting shows a blank screen

#### Scenario: Keep history
- **WHEN** a pane with scrollback receives `ClearPane{history: false}`
- **THEN** its live screen is blank and its cursor is at (0, 0), and scrolling up still shows the retained history

#### Scenario: Continue the program
- **WHEN** a program waiting for input has its pane cleared
- **THEN** it receives no bytes from clearing, remains running, and its next output starts at the first cell

#### Scenario: Clear an already empty pane
- **WHEN** a viewer clears an already empty pane
- **THEN** every viewer receives a full blank frame

#### Scenario: Unknown pane
- **WHEN** a client sends `ClearPane` for a nonexistent pane
- **THEN** it receives `Error{message: "no such pane: <pane_id>"}` and remains connected
