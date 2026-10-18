## ADDED Requirements

### Requirement: Signals are held across the exec
SIGUSR1 and SIGHUP arriving between the start of a reload and the successor installing its handlers SHALL NOT kill the supervisor; a SIGUSR1 in that window MUST be handled by the successor as a reload request.

#### Scenario: SIGHUP right after the exec
- **WHEN** the supervisor receives SIGHUP while its successor is starting
- **THEN** the server and its panes keep running

#### Scenario: SIGUSR1 right after the exec
- **WHEN** the supervisor receives SIGUSR1 while its successor is starting
- **THEN** the server keeps running and does one more reload

### Requirement: A refused reload keeps the socket serving
When a reload is refused or fails before the handover, the Unix socket and its lock SHALL stay in place and keep serving, so that `tilr` MUST NOT start a second server.

#### Scenario: Successor fails its check
- **WHEN** a reload is refused because the successor fails the reload check
- **THEN** `tilr ls` connects to the same server and no second `tilrd` runs
