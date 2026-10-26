## ADDED Requirements

### Requirement: A failed exec leaves SIGPIPE ignored
When the exec of a reload fails, in the supervisor or in a pane process, the process SHALL carry on with SIGPIPE ignored as before the reload, so a later write to a pipe or socket whose reader is gone MUST fail with an error and MUST NOT kill it.

**Verified by**: `reload::a_broken_pipe_after_a_failed_exec_does_not_kill_the_server`.

#### Scenario: A broken pipe after a failed exec
- **WHEN** a reload's exec fails in `execvp` (`reload-exec-missing=error`) and the server then logs to a stderr pipe whose reader has closed
- **THEN** the server keeps running, the same connection works and its pane keeps its shell

### Requirement: A stop asked for while a reload prepares its exec is not lost
A SIGTERM or SIGINT arriving after the supervisor has read its events a last time and before its exec SHALL stop the server: the successor SHALL shut down cleanly (exit status 0, `shutting down` in the log), or, if the exec fails, the server itself.

**Verified by**: `reload::a_sigterm_while_the_reload_prepares_its_exec_stops_the_server`.

#### Scenario: SIGTERM while the panes detach
- **WHEN** the supervisor receives SIGTERM after its last read of events and before the exec (`reload-after-drain=sleep:1500`, once the log has `failpoint reload-after-drain: sleeping`)
- **THEN** the exec goes ahead, the successor logs `SIGTERM came during the reload`, exits with status 0, and the shells end

### Requirement: A child that failed to start does not hold up a reload
A reload SHALL NOT wait for a child whose start ended in a panic: the next reload SHALL go through. If the server's threads cannot be made to hold the reload signals within 5 seconds, the reload SHALL fail with `reload failed: ... the signals cannot be held` in the log and the server SHALL carry on as before.

**Verified by**: `cloud::a_panic_starting_cloudflared_does_not_hold_up_the_next_reload`.

#### Scenario: A panic starting cloudflared
- **WHEN** starting `cloudflared` panics (`spawn-unblocked=panic`) and the server is then sent SIGUSR1
- **THEN** the log has `resumed after reload` and no `reload failed`, and the successor starts `cloudflared` again
