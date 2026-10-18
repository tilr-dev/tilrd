## ADDED Requirements

### Requirement: Hello within 10 seconds
A connection that has not sent `Hello` within 10 s of completing its WebSocket handshake SHALL be closed.

#### Scenario: WebSocket without Hello
- **WHEN** a client completes the WebSocket handshake and sends nothing
- **THEN** the server closes the connection after about 10 s

### Requirement: Un-greeted connections are capped
The server SHALL hold at most 128 connections that have not sent `Hello`; a connection beyond that MUST be closed at once, and greeted clients MUST keep working.

#### Scenario: Flood of silent connections
- **WHEN** 1000 TCP connections open and stay silent
- **THEN** the server holds at most 128 of them and an existing client keeps getting frames
