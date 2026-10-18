## MODIFIED Requirements

### Requirement: At most 64 streams per client
A client SHALL have at most 64 streams open at once; a program that connects while the serving client has 64 open MUST have its connection closed at once, and no `StreamOpen` is sent.

**Verified by**: `agent::a_65th_stream_is_refused` (planned in change test-coverage-gaps).

#### Scenario: The 65th connection
- **WHEN** a client has 64 agent streams open and a program connects to a socket it would serve
- **THEN** the program's connection is closed and the client gets no `StreamOpen`
