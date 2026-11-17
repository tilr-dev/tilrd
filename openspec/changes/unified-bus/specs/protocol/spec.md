## MODIFIED Requirements

### Requirement: One packet per binary WebSocket message
Every message between a WebSocket participant and the server SHALL travel as one binary WebSocket message: a flag byte, then a protobuf `Envelope` of `crates/tilr-bus/proto/bus.proto` (package `tilr.bus.v1`), on a connection whose handshake agreed on the subprotocol `tilr.bus.v1`. Flag `0` MUST mean a plain body and flag `1` a zstd-compressed body. The same protocol MUST be served on the TCP port and on the Unix socket, on every request path except the SOCKS path (see the `socks-proxy` spec) and `/metrics`.

**Verified by**: unit tests `codec::tests::small_envelopes_stay_plain`, `codec::tests::large_envelopes_are_compressed` and `codec::tests::rejects_garbage` (crates/tilr-bus/src/codec.rs), and `local::the_socket_is_the_users_alone_and_asks_no_token`.

#### Scenario: Flag byte and body
- **WHEN** either side sends a message
- **THEN** the WebSocket message is one binary frame whose first byte is `0` or `1`, followed by the protobuf `Envelope` as that flag says

#### Scenario: Same protocol on the Unix socket
- **WHEN** a local client opens a WebSocket with subprotocol `tilr.bus.v1` on the Unix socket at any URL (the client library uses `ws://localhost/`)
- **THEN** it speaks the same `Hello` / `HelloAck` as on the TCP port

### Requirement: Compression of large bodies
The sender SHALL compress an envelope of 256 bytes or more with zstd (level 1) and send it with flag `1`, unless the payload it carries is already compressed (`Chunk.content_encoding` `zstd`), and SHALL send other envelopes plain with flag `0`. The receiver MUST accept either flag on any message and MUST stop decompressing a body at `HelloAck.max_frame` (16 MiB).

**Verified by**: unit tests `codec::tests::small_envelopes_stay_plain`, `codec::tests::large_envelopes_are_compressed` and `codec::tests::a_compressed_chunk_is_not_compressed_again` (crates/tilr-bus/src/codec.rs).

#### Scenario: Small message stays plain
- **WHEN** an encoded envelope is under 256 bytes (a keystroke, its reply)
- **THEN** its message starts with flag `0` and the body is the plain protobuf

#### Scenario: Large message is compressed
- **WHEN** an encoded envelope is 256 bytes or more and carries no compressed payload
- **THEN** its message starts with flag `1`, is smaller than the plain body, and decodes back to the same envelope

#### Scenario: A frame compressed by its pane
- **WHEN** a `pane.output` chunk carries a frame the pane process compressed
- **THEN** its message starts with flag `0` and the chunk's payload is the pane's bytes unchanged

### Requirement: A malformed packet gets an error and the connection stays
When a WebSocket message cannot be decoded (empty, an unknown flag byte, bad zstd, bad protobuf), the server SHALL answer `ConnectionError` with code `bad_frame` and a `message` starting `bad packet: `, and MUST keep the connection open.

**Verified by**: `panes::a_malformed_packet_gets_an_error_and_the_connection_stays`.

#### Scenario: Unknown flag byte
- **WHEN** a greeted client sends the binary message `[9, 1, 2]`
- **THEN** it receives `ConnectionError` whose `message` contains `bad packet`
- **AND** a following call to `pane.list` is answered as usual

### Requirement: Unknown message types are ignored
Both sides SHALL ignore an `Envelope` whose variant they do not know, and fields they do not know, without answering an error, so that new envelope variants and fields can be added without breaking older peers. A call to a method nobody provides SHALL be answered `not_found`.

**Verified by**: `panes::a_message_type_the_server_does_not_know_is_ignored`.

#### Scenario: Field the server does not know
- **WHEN** a client sends an `Envelope` holding only field 99, then calls `pane.list`
- **THEN** the very next envelope it receives is the reply to `pane.list`, with no error before it

### Requirement: Hello comes first
`Hello` MUST be the first envelope a participant sends on a connection. The server SHALL answer any other envelope sent before `Hello` with `ConnectionError { code: "expected_hello", message: "expected Hello first" }` and close the connection, and MUST NOT act on that envelope.

**Verified by**: `panes::a_client_that_skips_hello_is_turned_away`.

#### Scenario: Input before Hello
- **WHEN** a client calls `pane.input` for an existing pane without saying `Hello`
- **THEN** it receives `ConnectionError` "expected Hello first" and then the connection closes
- **AND** the input never reaches the pane's shell

### Requirement: Protocol version check
`Hello.protocol_version` SHALL be the version of `tilr.bus.v1` the participant was built against (currently `1`). A server receiving any other value (older, newer, or 0 for none) MUST answer `ConnectionError { code: "unsupported_version", message: "unsupported protocol version <N>: this server speaks <version>" }` and close the connection, and MUST go on serving participants of its own version.

**Verified by**: `panes::a_client_built_for_another_protocol_version_is_turned_away`.

#### Scenario: Newer client
- **WHEN** a client sends `Hello` with `protocol_version` 2 to a server speaking 1
- **THEN** the first envelope it receives is `ConnectionError` "unsupported protocol version 2: this server speaks 1"
- **AND** nothing else follows: the connection is closed

#### Scenario: No version
- **WHEN** a client sends `Hello` without `protocol_version` (0)
- **THEN** the first envelope it receives is `ConnectionError` "unsupported protocol version 0: this server speaks 1"
- **AND** nothing else follows: the connection is closed

### Requirement: HelloAck is the first thing a client hears
The server SHALL send nothing to a connection before its `HelloAck`: events MUST skip a connection that has not said `Hello`. After `HelloAck` the connection receives the events it subscribes to.

**Verified by**: `panes::the_first_thing_a_client_hears_is_the_answer_to_its_hello`.

#### Scenario: Panes change before Hello
- **WHEN** a client connects, panes are created and retitled meanwhile, and the client then sends `Hello` and subscribes to `pane.*`
- **THEN** the first envelope it receives is `HelloAck`
- **AND** later pane changes reach it as `pane.*` events

### Requirement: HelloAck describes the server
`HelloAck` SHALL carry `protocol_version`, the connection's `identity`, `connection_id`, `max_frame`, `max_in_flight`, `stream_window`, `connection_window` and `server_id`. `connection_id` MUST differ per connection and grow across reconnects and reloads; a client's `seq_base` is `connection_id << 32` (see `screen-frames`). `server.info` SHALL carry `server_version`, `update_channel` (the config channel, default `stable`), `features`, `clipboard_max_bytes`, `fs_max_bytes` and `server_id`; limits MUST be 0 without their feature; `server_id` MUST be a UUID persisted in `server-id`. The channel MUST NOT change the protocol version.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `panes::a_server_keeps_its_id_when_it_restarts`, `metrics::a_client_that_reconnects_tells_its_input_from_before` and `panes::hello_reports_the_configured_update_channel`.

#### Scenario: Clipboard limit
- **WHEN** a server started with `--clipboard-max 3MB` is asked `server.info`
- **THEN** `clipboard_max_bytes` is 3145728

#### Scenario: Server id across restarts
- **WHEN** a server is killed and started again with the same state directory
- **THEN** `server_id` in `HelloAck` and `server.info` is the same UUID as before, and another server's is different

#### Scenario: Seq base per connection
- **WHEN** a client reconnects to the same server
- **THEN** the new connection's `connection_id` is greater than the old one's

#### Scenario: Configured update channel
- **WHEN** a server with `update_channel = "beta"` in `server.toml` is asked `server.info`
- **THEN** `update_channel` is `beta` and `HelloAck.protocol_version` remains 1

#### Scenario: Default update channel
- **WHEN** a server whose `server.toml` has no `update_channel` key is asked `server.info`
- **THEN** `update_channel` is `stable` and `HelloAck.protocol_version` remains 1

### Requirement: Server feature flags
`server.info.features` SHALL list exactly the capabilities this server can be run without and offers, in this order: `"clipboard"` (off: `--no-clipboard`), `"files"` (`--no-files` / `[files] enabled = false`), `"host-events"` (host-events socket not up), `"agent"` (`--no-agent` / `agent = false`), `"exec"` (`--no-exec` / `exec = false`), `"sandbox"` (the host cannot enforce sandboxed panes). What every server of the version has (`bus.ping`, the bus, profiles) MUST NOT be listed. The methods of a feature not listed MUST NOT be provided.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `clipboard::the_clipboard_can_be_turned_off`, `files::files_can_be_turned_off`, `agent::the_agent_is_forwarded_unless_turned_off`, `exec::exec_is_offered_unless_turned_off` and `sandbox::sandbox_is_offered_where_it_can_be_enforced`.

#### Scenario: Everything on
- **WHEN** a server started with default options on a host that can enforce sandboxes is asked `server.info`
- **THEN** `features` is `["clipboard", "files", "host-events", "agent", "exec", "sandbox"]`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec`
- **THEN** `"exec"` is missing from `features`, the rest are listed as before, and `exec.run` is answered `not_found`

### Requirement: Request ids and the general Error
A call SHALL be answered with the caller's `Call.id`; a failure SHALL be an error `Reply` with a `code` (see `component-bus`) and a `message` meant for the user, and MUST NOT close the connection; only a refused handshake (version, token, credential, `Hello` first) closes it, after a `ConnectionError`.

**Verified by**: `panes::asking_for_a_pane_that_does_not_exist_gets_an_error`.

#### Scenario: Unknown pane
- **WHEN** a client opens `pane.output` or calls `pane.input` naming a pane id that does not exist, or a name that is not a UUID such as `"1"`
- **THEN** it receives an error reply `not_found` with `no such pane: <name>` and the connection stays open

#### Scenario: Pushed pane list
- **WHEN** a pane appears, goes away or changes
- **THEN** every greeted participant subscribed to `pane.*` and allowed to access the pane receives `pane.created`, `pane.exited` or `pane.changed`

### Requirement: Version 1 is one contract
A server speaking `tilr.bus.v1` version 1 SHALL implement, without a feature flag, calls, events, streams with credits, `bus.ping`, `Hello.client_id` with superseding and close code 4000, close codes 4001, 4002 and 4003, `KeyInput.code`, the `*_bytes` twins of file paths, `PaneInfo.viewed_by` / `used_at_ms`, acknowledged input with replay, profiles and the bus's own methods; none of them SHALL appear in `server.info.features`.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`; each part is verified by the requirement that specifies it.

#### Scenario: A v1 client relies on the twins
- **WHEN** a client greeted with `protocol_version` 1 names a file by `path_bytes`
- **THEN** the server goes by the bytes, and `server.info.features` never lists `path-bytes`, `client-id`, `key-code`, `ping`, `bus`, `profiles` or input delivery

### Requirement: Handshake refusal closure permits WebSocket control frames
A participant observing a handshake refusal SHALL accept WebSocket Ping and Pong control frames between the `ConnectionError` and the connection closing. These frames MUST NOT count as closure or permit additional envelopes after the refusal.

#### Scenario: Liveness Ping between refusal Error and Close
- **WHEN** a client calls `pane.input` before Hello
- **AND** the server sends a liveness Ping after `ConnectionError` "expected Hello first" and before Close
- **THEN** the client accepts the control frame and continues waiting for closure
- **AND** the connection closes without another envelope

## REMOVED Requirements

### Requirement: Client feature flags
**Reason**: `Hello.features` is gone with `tilr.v1`: a participant serves what it provides (`host.open`, `ssh_agent.request`) and gets what it subscribes to (`pane.copied`).
**Migration**: `bus-terminal` "Host events and the SSH agent are calls the client serves" and "Copies and the shared clipboard on the bus".

### Requirement: Input is acknowledged after application
**Reason**: restated on the bus (the reply to `pane.input` / `pane.resize` is the acknowledgement; `pane.applied` replaces `HelloAck.applied_input`).
**Migration**: `bus-terminal` "Input is acknowledged after it is applied" and "Numbered input is applied once across reconnects"; tests unchanged by name.

### Requirement: Missing-pane input receives a terminal acknowledgement
**Reason**: a `not_found` reply to the call is the terminal answer; no separate `Error` + `InputAck` pair.
**Migration**: `bus-terminal` "Numbered input is applied once across reconnects".

### Requirement: Hello input probes are bounded
**Reason**: the probes run on `pane.applied`, not on `Hello`.
**Migration**: `bus-terminal` "Numbered input is applied once across reconnects" (5 s bound).

### Requirement: Input delivery identities have a bounded lifetime
**Reason**: `FinalizeInput` became `pane.input_finalize`; the rule is unchanged.
**Migration**: `bus-terminal` "Numbered input is applied once across reconnects".
