## ADDED Requirements

### Requirement: tilr checks who serves the socket
Before sending anything on the Unix socket, `tilr` SHALL check that the socket's directory belongs to its user, is 0700 and is not a symlink, and that the peer's uid is its own; otherwise it MUST refuse with an error naming the problem and exit non-zero.

#### Scenario: A socket served by another user
- **WHEN** the socket path is served by a process of another uid
- **THEN** `tilr ls` prints an error naming the other owner and exits non-zero without sending a Hello

### Requirement: The stored token goes to loopback targets only
`tilr` SHALL send the token read from the state directory only to a target on the loopback (`127.0.0.1`, `::1`, `localhost`); for another target it MUST send only a token given for that target.

#### Scenario: Remote target without its own token
- **WHEN** `tilr --connect ws://example.net:7890 ls` runs with a token in the state directory and none for that target
- **THEN** the Hello sent carries no token
