## ADDED Requirements

### Requirement: A failed exec leaves SIGPIPE ignored
When the exec of a reload fails, in the supervisor or in a pane process, the process SHALL carry on with SIGPIPE ignored as before the reload, so a later write to a pipe or socket whose reader is gone MUST fail with an error and MUST NOT kill it.

#### Scenario: A broken pipe after a failed exec
- **WHEN** a reload's exec fails and the server then writes to a pipe whose reader has closed
- **THEN** the server keeps running and its panes keep their shells

### Requirement: A stop asked for while a reload prepares its exec is not lost
A SIGTERM or SIGINT arriving after the supervisor has read its events a last time and before its exec SHALL stop the server: the successor SHALL shut down cleanly (exit status 0, `shutting down` in the log), or, if the exec fails, the server itself.

#### Scenario: SIGTERM while the panes detach
- **WHEN** the supervisor receives SIGTERM after its last read of events and before the exec (a failpoint `sleep:1500` there)
- **THEN** the successor logs `SIGTERM came during the reload`, exits with status 0, and the shells end

