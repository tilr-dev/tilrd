## ADDED Requirements

### Requirement: A client of the old protocol is told and closed
A WebSocket handshake that does not offer the subprotocol `tilr.bus.v1` SHALL be completed and answered with one `tilr.v1` packet `ServerMessage{error: Error{message: "this server speaks tilr.bus.v1; update the client"}}`, after which the server MUST close the connection with code 1002 and act on nothing the client sent.

**Verified by**: `bus::a_client_of_the_old_protocol_is_told_and_closed`.

#### Scenario: Old Hello
- **WHEN** a client connects without a subprotocol and sends the bytes of a `tilr.v1` `Hello`
- **THEN** it receives that packet, then a close frame with code 1002, and `tilr ls` shows no new pane or client
