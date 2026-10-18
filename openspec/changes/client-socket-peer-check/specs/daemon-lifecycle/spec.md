## ADDED Requirements

### Requirement: Lock and log files are not opened through links
The server and `tilr` SHALL open the socket's lock file and log file without following a symbolic link and MUST NOT truncate the lock file before holding the lock.

#### Scenario: Symlinked lock file
- **WHEN** `tilrd.sock.lock` is a symbolic link to another file
- **THEN** the server refuses to start with an error and the link target is unchanged
