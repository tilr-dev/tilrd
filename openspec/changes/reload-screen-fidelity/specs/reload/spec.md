## ADDED Requirements

### Requirement: Soft wraps carry over
A line soft-wrapped across the boundary between the history and the screen, and a pending wrap at the last column, SHALL be the same after a pane reload as before it.

**Verified by**: unit tests `ghostty::tests::snapshot_keeps_a_line_wrapped_across_the_history_border`, `ghostty::tests::snapshot_keeps_a_pending_wrap`; `scroll::a_line_wrapped_across_the_history_border_stays_one_line_after_a_reload`.

#### Scenario: Wrapped line at the border
- **WHEN** a long line wraps across the top of the screen into the history and the server reloads
- **THEN** copying the line from the history gives one line, as before the reload

#### Scenario: Pending wrap
- **WHEN** a program prints exactly as many characters as the pane has columns, the server reloads, and the program prints `x`
- **THEN** `x` is at the start of the next row and the last cell of the first row is unchanged

### Requirement: A closing pane is still killed after a reload
A pane whose program was sent `ClosePane` and has not exited SHALL still be killed at its deadline after a reload: the pane state SHALL carry the time left and the process groups being ended.

**Verified by**: `reload::a_closing_pane_is_still_killed_after_a_reload`.

#### Scenario: Reload during close
- **WHEN** a pane runs `sh -c 'trap "" HUP; sleep 1000'`, a client sends `ClosePane`, and the server reloads within 2 s
- **THEN** the `sleep` is killed when the deadline passes and the reconnected client gets `PaneExited` for the pane

### Requirement: Images in the history carry over
A kitty graphics placement whose top row is in the history SHALL be in the same place in the history after a pane reload.

**Verified by**: unit test `ghostty::tests::snapshot_keeps_an_image_scrolled_into_the_history`.

#### Scenario: Image scrolled partly off the screen
- **WHEN** an image is placed and output scrolls its top row into the history, and the server reloads
- **THEN** the first frame after the reload has the placement with the same negative top row, and scrolling back one row shows it whole
