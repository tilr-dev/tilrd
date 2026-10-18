## MODIFIED Requirements

### Requirement: Protocol version check
`Hello.protocol_version` SHALL be the `PROTOCOL_VERSION` the client was built against (currently `3`). A server receiving any other value (older, newer, or 0 for none) MUST answer `Error { message: "unsupported protocol version <N>: this server speaks <PROTOCOL_VERSION>" }` and close the connection, and MUST go on serving clients of its own version.

**Verified by**: `panes::a_client_built_for_another_protocol_version_is_turned_away`.

#### Scenario: Older client
- **WHEN** a client sends `Hello` with `protocol_version` 2 to a server speaking 3
- **THEN** the first message it receives is `Error` "unsupported protocol version 2: this server speaks 3"
- **AND** nothing else follows: the connection is closed

#### Scenario: Newer client
- **WHEN** a client sends `Hello` with `protocol_version` 4 to a server speaking 3
- **THEN** it is refused the same way, naming version 4

#### Scenario: No version
- **WHEN** a client sends `Hello` without `protocol_version` (0)
- **THEN** it is refused the same way, naming version 0

### Requirement: Server feature flags
`HelloAck.features` SHALL list exactly the capabilities this server can be run without and offers, in this order: `"clipboard"` (off: `--no-clipboard`), `"files"` (`--no-files` / `[files] enabled = false`), `"host-events"` (host-events socket not up), `"agent"` (`--no-agent` / `agent = false`), `"exec"` (`--no-exec` / `exec = false`). What every server of the version has (`Ping`) MUST NOT be listed. A client MUST NOT rely on a message of a feature not listed.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `clipboard::the_clipboard_can_be_turned_off`, `files::files_can_be_turned_off`, `agent::the_agent_is_forwarded_unless_turned_off` and `exec::exec_is_offered_unless_turned_off`.

#### Scenario: Everything on
- **WHEN** a server started with default options is greeted
- **THEN** `HelloAck.features` is `["clipboard", "files", "host-events", "agent", "exec"]`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec`
- **THEN** `"exec"` is missing from `HelloAck.features` and the rest are listed as before

## ADDED Requirements

### Requirement: Version 3 is one contract
A server speaking version 3 SHALL implement, without a feature flag, `Ping`, `Hello.client_id` with superseding and close code 4000, close codes 4001 and 4002, the bulk window paced by pongs, `KeyInput.code`, the `*_bytes` twins of file paths, and `PaneInfo.viewed_by` / `used_at_ms`; none of them SHALL appear in `HelloAck.features`.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit` (no `ping` in the list); each part is verified by the requirement that specifies it (`client-liveness`, `files`, `screen-frames`, `panes`).

#### Scenario: A v3 client relies on the twins
- **WHEN** a client greeted with `protocol_version` 3 names a file by `path_bytes`
- **THEN** the server goes by the bytes, and `HelloAck.features` never lists `path-bytes`, `client-id`, `key-code` or `ping`
