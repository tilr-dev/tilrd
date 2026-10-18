## ADDED Requirements

### Requirement: Soft wraps carry over
A line soft-wrapped across the boundary between the history and the screen, and a pending wrap at the last column, SHALL be the same after a pane reload as before it.

#### Scenario: Wrapped line at the border
- **WHEN** a long line wraps across the top of the screen into the history and the server reloads
- **THEN** copying the line from the history gives one line, as before the reload

### Requirement: A closing pane is still killed after a reload
A pane whose program was sent `ClosePane` and has not exited SHALL still be killed at its deadline after a reload.

#### Scenario: Reload during close
- **WHEN** a pane ignores SIGHUP after `ClosePane` and the server reloads within 2 s
- **THEN** the program is killed when the deadline passes
