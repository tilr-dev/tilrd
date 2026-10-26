## MODIFIED Requirements

### Requirement: Closing a pane ends its program
`ClosePane` SHALL send SIGHUP to the process group of the pane's program and to the pty's foreground process group and, if they have not emptied 2 s later, SIGKILL both; the pane then exits as any pane does.

**Verified by**: `panes::closing_a_pane_ends_its_shell`, `panes::closing_a_pane_ends_a_foreground_program_ignoring_sighup`.

#### Scenario: Close
- **WHEN** a client sends `ClosePane` for a pane running a shell
- **THEN** it gets `PaneExited` for that pane and `ListPanes` no longer lists it

#### Scenario: Close with a foreground job ignoring SIGHUP
- **WHEN** a pane's shell runs `sh -c 'trap "" HUP; sleep 1000'` in the foreground and a client sends `ClosePane`
- **THEN** within 3 s the client gets `PaneExited` and the `sleep` is no longer running
