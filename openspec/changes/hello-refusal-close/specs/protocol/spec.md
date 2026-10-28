## ADDED Requirements

### Requirement: Handshake refusal closure permits WebSocket control frames
A client observing a handshake refusal SHALL accept WebSocket Ping and Pong control frames between the protocol Error and the connection closing. These frames MUST NOT count as closure or permit additional application messages after the refusal Error.

#### Scenario: Liveness Ping between refusal Error and Close
- **WHEN** a client sends numbered input before Hello
- **AND** the server sends a liveness Ping after Error "expected Hello first" and before Close
- **THEN** the client accepts the control frame and continues waiting for closure
- **AND** the connection closes without another application message
