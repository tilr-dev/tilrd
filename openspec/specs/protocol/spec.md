# protocol Specification

## Purpose

Defines the client wire protocol of `tilrd`: how packets travel over the WebSocket, the `Hello` / `HelloAck` handshake, protocol version checks, the feature flags each side announces, and the general `Error` message and request ids. Who may connect lives in the `access-control` spec, pings, close codes and slow clients in `client-liveness`, frame content in `screen-frames`, and each feature's messages in its own spec (`files`, `exec`, `ssh-agent-forwarding`, `clipboard-host-events`).

## Requirements

### Requirement: One packet per binary WebSocket message
Every client and server message SHALL travel as one binary WebSocket message: a flag byte, then a protobuf `ClientMessage` (client to server) or `ServerMessage` (server to client) from `crates/tilr-proto/proto/tilr.proto` (package `tilr.v1`). Flag `0` MUST mean a plain body and flag `1` a zstd-compressed body. The same protocol MUST be served on the TCP port and on the Unix socket, on every request path except the SOCKS path (see the `socks-proxy` spec).

**Verified by**: unit tests `small_message_stays_plain`, `large_message_is_compressed` and `rejects_garbage` in crates/tilr-proto/src/lib.rs, and by `local::the_socket_is_the_users_alone_and_asks_no_token`.

#### Scenario: Flag byte and body
- **WHEN** either side sends a message
- **THEN** the WebSocket message is one binary frame whose first byte is `0` or `1`, followed by the protobuf body as that flag says

#### Scenario: Same protocol on the Unix socket
- **WHEN** a local client opens a WebSocket on the Unix socket at any URL (the client library uses `ws://localhost/`)
- **THEN** it speaks the same `Hello` / `HelloAck` protocol as on the TCP port

### Requirement: Compression of large bodies
The sender SHALL compress a body of 256 bytes or more with zstd (level 1) and send it with flag `1`; a body under 256 bytes MUST go plain with flag `0`. The receiver MUST accept either flag on any message and MUST stop decompressing a body at 64 MiB.

**Verified by**: unit tests `small_message_stays_plain` and `large_message_is_compressed` in crates/tilr-proto/src/lib.rs.

#### Scenario: Small message stays plain
- **WHEN** an encoded message is under 256 bytes (a keystroke, a one-row diff)
- **THEN** its packet starts with flag `0` and the body is the plain protobuf

#### Scenario: Large message is compressed
- **WHEN** an encoded message is 256 bytes or more (a full frame)
- **THEN** its packet starts with flag `1`, is smaller than the plain body, and decodes back to the same message

### Requirement: A malformed packet gets an error and the connection stays
When a packet cannot be decoded (empty, an unknown flag byte, bad zstd, bad protobuf), the server SHALL answer `Error` with a `message` starting `bad packet: ` and MUST keep the connection open.

**Verified by**: `panes::a_malformed_packet_gets_an_error_and_the_connection_stays`.

#### Scenario: Unknown flag byte
- **WHEN** a client sends the binary message `[9, 1, 2]`
- **THEN** it receives `Error` whose `message` contains `bad packet`
- **AND** a following `ListPanes` is answered as usual

### Requirement: Unknown message types are ignored
Both sides SHALL ignore a message whose type they do not know (a `ClientMessage` / `ServerMessage` with no `msg` field they recognise) without answering an error, so that new message types can be added without breaking older peers.

**Verified by**: `panes::a_message_type_the_server_does_not_know_is_ignored`.

#### Scenario: Field the server does not know
- **WHEN** a client sends a `ClientMessage` holding only field 99, then `ListPanes { request_id: 1 }`
- **THEN** the very next message it receives is `PaneList` with `request_id` 1, with no `Error` before it

### Requirement: Hello comes first
`Hello` MUST be the first message a client sends on a connection. The server SHALL answer any other known message sent before `Hello` with `Error { message: "expected Hello first" }` and close the connection, and MUST NOT act on that message.

**Verified by**: `panes::a_client_that_skips_hello_is_turned_away`.

#### Scenario: Input before Hello
- **WHEN** a client sends `KeyInput` / `RawInput` for an existing pane without saying `Hello`
- **THEN** it receives `Error` "expected Hello first" and then the connection closes
- **AND** the input never reaches the pane's shell

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

### Requirement: HelloAck is the first thing a client hears
The server SHALL send nothing to a connection before its `HelloAck`: `PaneList` pushes and other broadcasts MUST skip a connection that has not said `Hello`. After `HelloAck` the connection receives pushes like every other client.

**Verified by**: `panes::the_first_thing_a_client_hears_is_the_answer_to_its_hello`.

#### Scenario: Panes change before Hello
- **WHEN** a client connects, panes are created and retitled meanwhile, and the client then sends `Hello`
- **THEN** the first message it receives is `HelloAck`
- **AND** later pane changes reach it as pushed `PaneList` messages

### Requirement: HelloAck describes the server
`HelloAck` SHALL carry `protocol_version` (the server's `PROTOCOL_VERSION`), `server_version` (the build), `features`, `clipboard_max_bytes` (0 without `"clipboard"`), `server_id`, `fs_max_bytes` (0 without `"files"`) and `seq_base`. `server_id` MUST be a UUID kept in `server-id` in the state directory, the same after a restart. `seq_base` MUST differ for every connection, its connection number in the high 32 bits (its use is in `screen-frames`).

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `panes::a_server_keeps_its_id_when_it_restarts` and `metrics::a_client_that_reconnects_tells_its_input_from_before`.

#### Scenario: Clipboard limit
- **WHEN** a server started with `--clipboard-max 3MB` is greeted
- **THEN** `HelloAck.protocol_version` is `PROTOCOL_VERSION` and `clipboard_max_bytes` is 3145728

#### Scenario: Server id across restarts
- **WHEN** a server is killed and started again with the same state directory
- **THEN** `HelloAck.server_id` is the same UUID as before, and another server's is different

#### Scenario: Seq base per connection
- **WHEN** a client reconnects to the same server
- **THEN** the new connection's `seq_base` is greater than the old one's

### Requirement: Server feature flags
`HelloAck.features` SHALL list exactly the capabilities this server can be run without and offers, in this order: `"clipboard"` (off: `--no-clipboard`), `"files"` (`--no-files` / `[files] enabled = false`), `"host-events"` (host-events socket not up), `"agent"` (`--no-agent` / `agent = false`), `"exec"` (`--no-exec` / `exec = false`). What every server of the version has (`Ping`) MUST NOT be listed. A client MUST NOT rely on a message of a feature not listed.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `clipboard::the_clipboard_can_be_turned_off`, `files::files_can_be_turned_off`, `agent::the_agent_is_forwarded_unless_turned_off` and `exec::exec_is_offered_unless_turned_off`.

#### Scenario: Everything on
- **WHEN** a server started with default options is greeted
- **THEN** `HelloAck.features` is `["clipboard", "files", "host-events", "agent", "exec"]`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec`
- **THEN** `"exec"` is missing from `HelloAck.features` and the rest are listed as before

### Requirement: Client feature flags
The server SHALL read `Hello.features` for the capabilities that change what it sends a client: `"host-events"` (the client is sent `HostEvent` and counts as a listener for `tilr edit` / `tilr open`) and `"agent"` (the client can serve the SSH agent of the panes it watches). Any other entry MUST be ignored.

**Verified by**: `files::tilr_edit_fails_with_no_client_watching_or_outside_a_pane` and `agent::a_program_talks_to_the_agent_of_the_client_through_the_panes_socket`.

#### Scenario: Client without host-events
- **WHEN** a client whose `Hello.features` is `["clipboard"]` watches a pane and a program in it runs `tilr edit notes.md`
- **THEN** that client is not sent a `HostEvent` and does not count as a client attached to the pane

### Requirement: Request ids and the general Error
The server SHALL echo a client-chosen `request_id`: `CreatePane`'s in `PaneCreated`, `ListPanes`'s in `PaneList`; a `PaneList` pushed on its own MUST carry `request_id` 0. A failure of a request without an error field of its own SHALL be reported as `Error { message }`, meant for the user, and MUST NOT close the connection unless it refuses the handshake (version, token, `Hello` first). `Fs*`, `Exec*` and `ClipboardPut` report failures in their own answers.

**Verified by**: `panes::asking_for_a_pane_that_does_not_exist_gets_an_error`.

#### Scenario: Unknown pane
- **WHEN** a client sends `Subscribe` or `KeyInput` naming a pane id that does not exist, or a name that is not a UUID such as `"1"`
- **THEN** it receives `Error { message: "no such pane: <name>" }` and the connection stays open

#### Scenario: Pushed pane list
- **WHEN** a pane appears, goes away or changes
- **THEN** every greeted client receives `PaneList` with `request_id` 0

### Requirement: Version 3 is one contract
A server speaking version 3 SHALL implement, without a feature flag, `Ping`, `Hello.client_id` with superseding and close code 4000, close codes 4001 and 4002, the bulk window paced by pongs, `KeyInput.code`, the `*_bytes` twins of file paths, and `PaneInfo.viewed_by` / `used_at_ms`; none of them SHALL appear in `HelloAck.features`.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit` (no `ping` in the list); each part is verified by the requirement that specifies it (`client-liveness`, `files`, `screen-frames`, `panes`).

#### Scenario: A v3 client relies on the twins
- **WHEN** a client greeted with `protocol_version` 3 names a file by `path_bytes`
- **THEN** the server goes by the bytes, and `HelloAck.features` never lists `path-bytes`, `client-id`, `key-code` or `ping`

## Notes

- Requirements without a test: none. Partly untested: the 64 MiB decompression cap of "Compression of large bodies".
- Unit tests in crates/tilr-proto/src/lib.rs are cited by bare name (they are not in a `tests/` file).
- Code vs README: README "Protocol" says a server "sent anything else first answers with an `Error` and closes the connection". In code (crates/tilrd/src/supervisor.rs:1846-1851) a packet that does not decode, or decodes to an unknown message type, is handled before the `Hello` check: before `Hello` it gets `bad packet: …` (connection kept) or is ignored, not refused. Specified as the code does.
- `"agent"` in `HelloAck.features` is listed whenever forwarding is on, whatever the client's `Hello.features`; what the client's `"agent"` gates is `StreamOpen` (see `ssh-agent-forwarding`).
- The rule for later changes (README "Protocol", the tilr.proto header, openspec/config.yaml): a change a peer cannot ignore bumps `PROTOCOL_VERSION`; a feature names only a capability a server can be run without; a change a peer may ignore needs neither. A process rule, not an observable behavior.
- File names that are not UTF-8 (`*_bytes` twins) are a rule of the files protocol only: see the `files` spec.
- The supervisor-to-pane-process framing (u32 LE length, kind byte; crates/tilrd/src/link.rs) is internal and not part of the client protocol.
