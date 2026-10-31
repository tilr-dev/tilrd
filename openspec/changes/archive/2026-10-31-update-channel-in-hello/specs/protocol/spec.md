## MODIFIED Requirements

### Requirement: HelloAck describes the server
`HelloAck` SHALL carry `protocol_version` (server `PROTOCOL_VERSION`), `server_version` (build), `update_channel` (config channel; default `stable`), `features`, `clipboard_max_bytes`, `server_id`, `fs_max_bytes` and `seq_base`. Limits MUST be 0 without their feature. `server_id` MUST be a UUID persisted in `server-id`. `seq_base` MUST differ per connection, its connection number in the high 32 bits (see `screen-frames`). The channel field MUST NOT change the protocol version.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `panes::a_server_keeps_its_id_when_it_restarts`, `metrics::a_client_that_reconnects_tells_its_input_from_before` and `panes::hello_reports_the_configured_update_channel`.

#### Scenario: Clipboard limit
- **WHEN** a server started with `--clipboard-max 3MB` is greeted
- **THEN** `HelloAck.protocol_version` is `PROTOCOL_VERSION` and `clipboard_max_bytes` is 3145728

#### Scenario: Server id across restarts
- **WHEN** a server is killed and started again with the same state directory
- **THEN** `HelloAck.server_id` is the same UUID as before, and another server's is different

#### Scenario: Seq base per connection
- **WHEN** a client reconnects to the same server
- **THEN** the new connection's `seq_base` is greater than the old one's

#### Scenario: Configured update channel
- **WHEN** a server with `update_channel = "beta"` in `server.toml` is greeted
- **THEN** `HelloAck.update_channel` is `beta` and `protocol_version` remains `PROTOCOL_VERSION`

#### Scenario: Default update channel
- **WHEN** a server whose `server.toml` has no `update_channel` key is greeted
- **THEN** `HelloAck.update_channel` is `stable` and `protocol_version` remains `PROTOCOL_VERSION`
