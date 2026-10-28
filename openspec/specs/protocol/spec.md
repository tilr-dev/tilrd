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

### Requirement: Version 1 is one contract
A server speaking version 1 SHALL implement, without a feature flag, `Ping`, `Hello.client_id` with superseding and close code 4000, close codes 4001 and 4002, the bulk window paced by pongs, `KeyInput.code`, the `*_bytes` twins of file paths, `PaneInfo.viewed_by` / `used_at_ms`, and acknowledged input with replay; none of them SHALL appear in `HelloAck.features`.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`; each part is verified by the requirement that specifies it.

#### Scenario: A v1 client relies on the twins
- **WHEN** a client greeted with `protocol_version` 1 names a file by `path_bytes`
- **THEN** the server goes by the bytes, and `HelloAck.features` never lists `path-bytes`, `client-id`, `key-code`, `ping` or input delivery

### Requirement: Input is acknowledged after application
Clients SHALL number input per identity and pane and retain it until ACK for ordered replay. The server SHALL ACK completed PTY writes independently of output, deduplicate accepted sequences and report applied counters in Hello. Deduplication ends at finalization, one hour without reconnect after disconnect, or pane exec; later replay MAY duplicate input. Delivery and latency numbering SHALL be independent. Outboxes SHALL reject overflow before sending or advancing counters.

**Verified by**: `input_delivery::numbered_input_requires_a_stable_client_id`, `input_delivery::disconnect_before_application_replays_one_copy`, `input_delivery::applied_input_with_a_lost_ack_is_not_applied_twice`, `input::tests::overflow_is_atomic_and_ack_reclaims_capacity`, `client::attach_replays_a_key_when_the_connection_ends_without_an_ack`, `session::input_delivery_tests::reconnect_replays_only_unacknowledged_input_in_order`.

#### Scenario: Application without screen output
- **WHEN** input is written to a PTY whose program emits no output
- **THEN** the client receives an InputAck and can remove that input from its queue

#### Scenario: Lost acknowledgement
- **WHEN** a connection ends after input is applied but before its acknowledgement arrives, reconnects within one hour, and no pane exec occurred
- **THEN** HelloAck reports the applied sequence and replay cannot apply the input again

#### Scenario: Pending duplicate
- **WHEN** a frame is replayed while some of its bytes are still pending
- **THEN** only the original pending bytes are written and the frame is acknowledged after their final byte

#### Scenario: Numbered input without a stable identity
- **WHEN** a greeted client without `Hello.client_id` sends input with a nonzero `input_seq`
- **THEN** the server returns `Error "numbered input requires a stable client_id"` and does not apply that input

#### Scenario: Full outbox
- **WHEN** another input would exceed 4 MiB or 4096 queued frames
- **THEN** admission fails visibly, the frame is not sent and existing queued input remains available for replay

### Requirement: Missing-pane input receives a terminal acknowledgement
Numbered input from a greeted client with a stable identity naming no running pane SHALL receive `Error { message: "no such pane: <name as sent>" }` then `InputAck { pane_id: <name as sent>, seq: <input_seq> }`. Clients SHALL reclaim that exact pane string's queued input through the acknowledged sequence. Rejection MUST NOT change applied counters. Unnumbered requests SHALL receive only the error.

**Verified by**: `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect`.

#### Scenario: Rejected numbered input
- **WHEN** a stable identified client sends numbered key, mouse, paste, raw, scroll or resize input naming an absent UUID or malformed pane name
- **THEN** it receives the error and terminal acknowledgement, can reclaim that pane's queued input and keeps other panes' pending input
- **AND** the acknowledgement terminates delivery without reporting application to a PTY or changing a live pane's applied counter

#### Scenario: Lost rejection acknowledgement
- **WHEN** the rejection acknowledgement is lost and the client replays after reconnect or reload while the pane remains absent
- **THEN** the server returns the same error and acknowledgement without adding an applied counter to HelloAck

#### Scenario: Acknowledged rejection does not replay
- **WHEN** the client reconnects after consuming the rejection acknowledgement
- **THEN** its outbox no longer replays that rejected input

### Requirement: Hello input probes are bounded
The server SHALL give pane processes five seconds to answer Hello input probes,
checked on each supervisor tick. A pane process that has not answered within that
bound SHALL be killed and its pane closed. Hello SHALL complete after those pane
links close, retaining the applied input counters from responsive panes. A client
that has sent a valid Hello SHALL NOT be closed by the timeout for missing Hello
while its probes are pending.

**Verified by**: `input_delivery::a_stuck_pane_does_not_block_other_panes_hello`.

#### Scenario: A pane stops reading its input pipe
- **WHEN** two stable clients send Hello while a pane process is stopped before
  answering its input probe
- **THEN** the unresponsive pane is closed within the probe bound
- **AND** both clients receive HelloAck with applied counters for responsive panes
- **AND** input on responsive panes continues without duplicate application

### Requirement: Input delivery identities have a bounded lifetime
Clients SHALL finalize only after all admitted input is acknowledged. The server SHALL remove finalized identities immediately and disconnected identities after one hour without reconnect. Reconnect before expiry SHALL cancel the deadline. Cleanup SHALL preserve queued PTY bytes. Neither delivery counters nor disconnect deadlines SHALL survive exec. Protocol version SHALL remain 1.

**Verified by**: `input_delivery::finalized_sessions_do_not_accumulate_delivery_history`, `input_delivery::expired_sessions_are_removed_but_active_sessions_are_retained`, `input_delivery::expiry_preserves_pending_bytes_without_stale_ack_markers`, `input_delivery::clients_cannot_forge_internal_delivery_cleanup`, `supervisor::tests::input_retention_expires_at_one_hour_and_reconnect_cancels_it`, `client::normal_detach_finalizes_its_acknowledged_input`, `session::input_delivery_tests::normal_disconnect_drains_input_before_finalizing`.

#### Scenario: Many completed client sessions
- **WHEN** successive identities stop admitting input, receive all acknowledgements, send `FinalizeInput` and close
- **THEN** their delivery records SHALL be removed immediately, further numbered input on the finalized connection SHALL be refused, and later Hello SHALL report no applied input for them

#### Scenario: Disconnect retention and expiry
- **WHEN** an identity disconnects without finalizing
- **THEN** reconnect before one hour retains its applied counter and cancels expiry; reconnect after an uninterrupted hour starts with no applied counter and may replay already applied input

#### Scenario: Pending bytes outlive history
- **WHEN** an expired identity has bytes still queued for its PTY
- **THEN** cleanup preserves the bytes and the pane keeps running when they are written

#### Scenario: Client cannot request another identity's cleanup
- **WHEN** a client supplies internal cleanup fields
- **THEN** ingress SHALL clear them and SHALL NOT remove any other identity

### Requirement: Handshake refusal closure permits WebSocket control frames
A client observing a handshake refusal SHALL accept WebSocket Ping and Pong control frames between the protocol Error and the connection closing. These frames MUST NOT count as closure or permit additional application messages after the refusal Error.

#### Scenario: Liveness Ping between refusal Error and Close
- **WHEN** a client sends numbered input before Hello
- **AND** the server sends a liveness Ping after Error "expected Hello first" and before Close
- **THEN** the client accepts the control frame and continues waiting for closure
- **AND** the connection closes without another application message

## Notes

- Requirements without a test: none. Partly untested: the 64 MiB decompression cap of "Compression of large bodies".
- Unit tests in crates/tilr-proto/src/lib.rs are cited by bare name (they are not in a `tests/` file).
- `"agent"` in `HelloAck.features` is listed whenever forwarding is on, whatever the client's `Hello.features`; what the client's `"agent"` gates is `StreamOpen` (see `ssh-agent-forwarding`).
- The rule for later changes (README "Protocol", the tilr.proto header, openspec/config.yaml): before the first public release, all protocol changes use version 1 without a bump. Afterward, bump `PROTOCOL_VERSION` only for a change incompatible with a client or server of the previous public release, once per release as a separate step in its release milestone. A feature names only a capability a server can be run without; a change a peer may ignore needs neither a bump nor a feature. A process rule, not an observable behavior.
- File names that are not UTF-8 (`*_bytes` twins) are a rule of the files protocol only: see the `files` spec.
- The supervisor-to-pane-process framing (u32 LE length, kind byte; crates/tilrd/src/link.rs) is internal and not part of the client protocol.
