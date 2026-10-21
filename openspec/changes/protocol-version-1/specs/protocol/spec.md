## RENAMED Requirements

- FROM: `### Requirement: Version 3 is one contract`
- TO: `### Requirement: Version 1 is one contract`

## MODIFIED Requirements

### Requirement: Protocol version check
`Hello.protocol_version` SHALL be the `PROTOCOL_VERSION` the client was built against (currently `1`). A server receiving any other value (older, newer, or 0 for none) MUST answer `Error { message: "unsupported protocol version <N>: this server speaks <PROTOCOL_VERSION>" }` and close the connection, and MUST go on serving clients of its own version.

**Verified by**: `panes::a_client_built_for_another_protocol_version_is_turned_away`.

#### Scenario: Newer client
- **WHEN** a client sends `Hello` with `protocol_version` 2 to a server speaking 1
- **THEN** the first message it receives is `Error` "unsupported protocol version 2: this server speaks 1"
- **AND** nothing else follows: the connection is closed

#### Scenario: No version
- **WHEN** a client sends `Hello` without `protocol_version` (0)
- **THEN** the first message it receives is `Error` "unsupported protocol version 0: this server speaks 1"
- **AND** nothing else follows: the connection is closed

### Requirement: Version 1 is one contract
A server speaking version 1 SHALL implement, without a feature flag, `Ping`, `Hello.client_id` with superseding and close code 4000, close codes 4001 and 4002, the bulk window paced by pongs, `KeyInput.code`, the `*_bytes` twins of file paths, and `PaneInfo.viewed_by` / `used_at_ms`; none of them SHALL appear in `HelloAck.features`.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit` (no `ping` in the list); each part is verified by the requirement that specifies it (`client-liveness`, `files`, `screen-frames`, `panes`).

#### Scenario: A v1 client relies on the twins
- **WHEN** a client greeted with `protocol_version` 1 names a file by `path_bytes`
- **THEN** the server goes by the bytes, and `HelloAck.features` never lists `path-bytes`, `client-id`, `key-code` or `ping`

