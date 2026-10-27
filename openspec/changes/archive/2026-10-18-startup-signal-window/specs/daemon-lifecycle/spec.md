## ADDED Requirements

### Requirement: Signals while the server starts
From the moment the server binds its TCP port or claims its Unix socket until its signal handlers are in place, SIGTERM, SIGINT, SIGHUP and SIGUSR1 MUST NOT end the server by their default action: each SHALL be recorded and acted on once the handlers are in place, SIGTERM and SIGINT as a stop (see Stopping: the Unix socket and its lock file removed, exit 0), SIGHUP ignored, SIGUSR1 one reload.

**Verified by**: `local::a_sigterm_while_the_server_starts_stops_it_cleanly`, `local::a_sigint_while_the_server_starts_stops_it_cleanly`, `local::a_sighup_while_the_server_starts_is_ignored`.

#### Scenario: SIGTERM between the bind and the handlers
- **WHEN** `tilrd` starts with the failpoint `supervisor-before-signals=sleep:1000`, its socket accepts a connection and it receives SIGTERM during the sleep
- **THEN** it exits 0, the log has `shutting down`, and neither `tilrd.sock` nor `tilrd.sock.lock` is left

#### Scenario: Ctrl-C between the bind and the handlers
- **WHEN** the same is done with SIGINT
- **THEN** it exits 0 and neither the socket nor the lock file is left

#### Scenario: SIGHUP between the bind and the handlers
- **WHEN** it receives SIGHUP during the sleep
- **THEN** it keeps running and serves a client after the sleep, and the log has `SIGHUP ignored`
