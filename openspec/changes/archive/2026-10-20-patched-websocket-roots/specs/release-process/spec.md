## ADDED Requirements

### Requirement: Outgoing WebSockets use patched bundled roots
tilrd SHALL verify outgoing secure WebSocket certificates and hostnames with the bundled Mozilla roots from webpki-roots >=1.0.10 and rustls-webpki >=0.103.16, without webpki-roots 0.26, native trust-store substitution or advisory ignores. Cloud control and `tilrd ps` SHALL use the same explicit rustls connector; plain WebSockets SHALL remain supported.

**Verified by**: unit tests in `crates/tilrd/src/websocket.rs` and `cargo deny check`.

#### Scenario: Secure WebSocket peer
- **WHEN** tilrd connects to a secure WebSocket
- **THEN** it verifies the peer against the bundled roots and rejects an untrusted issuer or a hostname mismatch

#### Scenario: Plain WebSocket peer
- **WHEN** tilrd connects to a plain WebSocket
- **THEN** it performs the WebSocket handshake without TLS and preserves request headers

#### Scenario: Dependency regression
- **WHEN** the dependency graph contains an affected roots or verifier release
- **THEN** the regression check or cargo deny fails instead of ignoring the advisory
