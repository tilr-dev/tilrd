## MODIFIED Requirements

### Requirement: The SOCKS WebSocket is on the TCP port only
`GET /socks` SHALL be served only on the TCP port; on the Unix socket a handshake for `/socks` MUST be refused with 404 whether the proxy is on or off. Any other path is the main protocol.

**Verified by**: `socks::the_socks_path_is_not_served_on_the_unix_socket` (planned in change server-requirement-tests).

#### Scenario: Over the Unix socket
- **WHEN** a client opens a WebSocket for `/socks` on the server's Unix socket of a server run with `--socks`
- **THEN** the handshake is refused with 404

### Requirement: The client must say where it goes within 10 s
A SOCKS session SHALL be given 10 s from its first byte to finish the negotiation and the request; one that has not MUST be closed. (The time to the first byte is the port's handshake timeout, see the `daemon-lifecycle` spec.)

**Verified by**: `socks::a_session_that_does_not_say_where_it_goes_is_closed` (planned in change server-requirement-tests).

#### Scenario: A client that stalls
- **WHEN** a raw SOCKS client sends its greeting and then nothing for 10 s
- **THEN** the server closes the connection
