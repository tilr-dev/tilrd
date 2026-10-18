## ADDED Requirements

### Requirement: The successor must accept the command line and configuration
Before a reload's exec the supervisor SHALL check that the executable on disk accepts every command-line flag the server was started with and the current configuration file, and MUST NOT exec into one that does not; it SHALL log the reason and keep running with its panes.

#### Scenario: Rollback to a release without --socket
- **WHEN** a server started by `tilr` with `--socket` and `--lock-fd` is sent SIGUSR1 after the executable was replaced by a release that does not know those flags
- **THEN** no exec happens, the server keeps its pid and panes
- **AND** the log names the flag the successor lacks

#### Scenario: Configuration the successor refuses
- **WHEN** the configuration file has a key the executable on disk rejects and the server gets SIGUSR1
- **THEN** no exec happens and the server keeps running

### Requirement: Signals are held across the exec
SIGUSR1 and SIGHUP arriving between the start of a reload and the successor installing its handlers SHALL NOT kill the supervisor; a SIGUSR1 in that window MUST be handled by the successor as a reload request.

#### Scenario: SIGHUP right after the exec
- **WHEN** the supervisor receives SIGHUP while its successor is starting
- **THEN** the server and its panes keep running

### Requirement: A refused reload keeps the socket serving
When a reload is refused or fails before the handover, the Unix socket and its lock SHALL stay in place and keep serving, so that `tilr` MUST NOT start a second server.

#### Scenario: Successor cannot take the socket
- **WHEN** a reload is refused because the successor cannot take the socket
- **THEN** `tilr ls` connects to the same server and no second `tilrd` runs
