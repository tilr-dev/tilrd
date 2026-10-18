## ADDED Requirements

### Requirement: A reload from the old runtime directory keeps the panes
A server resuming after a reload SHALL find its state in the runtime directory of the release it was reloaded from, including `$TMPDIR/tilr-<uid>` when no runtime variable is set, and MUST keep its panes and its socket.

#### Scenario: Linux without XDG_RUNTIME_DIR
- **WHEN** a server of a release that kept its state under `$TMPDIR/tilr-<uid>` reloads into this release
- **THEN** the server keeps its pid, panes and socket
