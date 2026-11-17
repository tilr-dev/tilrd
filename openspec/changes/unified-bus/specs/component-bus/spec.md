## ADDED Requirements

### Requirement: One protocol on every transport
The server SHALL speak `tilr.bus.v1` (`crates/tilr-bus/proto/bus.proto`) on the TCP port and on the Unix socket over WebSocket with the subprotocol `tilr.bus.v1`, one `Envelope` per binary message after a flag byte (`0` plain, `1` zstd), and on instance transports (descriptor 3, instance sockets) as a u32 big-endian length followed by an `Envelope`.

**Verified by**: `bus::every_transport_speaks_the_same_envelopes`, unit test `codec::tests::small_envelopes_stay_plain` (crates/tilr-bus/src/codec.rs).

#### Scenario: Same call on three transports
- **WHEN** the test runtime runs as an fd instance, a socket instance, and a WebSocket participant with a credential, and each calls `test.echo` with `x`
- **THEN** each gets `x` back

### Requirement: Envelopes are limited to 16 MiB
An `Envelope` longer than `HelloAck.max_frame` (16 MiB) after decompression, in either direction, MUST close the connection as a protocol error.

**Verified by**: `bus::a_frame_over_the_limit_closes_the_connection`, unit test `codec::tests::a_frame_over_the_limit_is_refused` (crates/tilr-bus/src/codec.rs).

#### Scenario: Oversized frame
- **WHEN** a socket instance sends a length prefix of 16 MiB + 1
- **THEN** its connection is closed and the instance is restarted

### Requirement: A client of the old protocol is told and closed
A WebSocket handshake that does not offer the subprotocol `tilr.bus.v1` SHALL be completed and answered with one `tilr.v1` packet `ServerMessage{error: Error{message: "this server speaks tilr.bus.v1; update the client"}}`, after which the server MUST close the connection with code 1002 and act on nothing the client sent.

**Verified by**: `bus::a_client_of_the_old_protocol_is_told_and_closed`.

#### Scenario: Old Hello
- **WHEN** a client connects without a subprotocol and sends the bytes of a `tilr.v1` `Hello`
- **THEN** it receives that packet, then a close frame with code 1002, and `tilr ls` shows no new pane or client

### Requirement: Hello comes first and names a version
`Hello` MUST be the first envelope on a connection, with `protocol_version` 1, and SHALL be answered with `HelloAck` (`protocol_version`, `identity`, `connection_id` growing across reloads, the limits, `server_id`, an instance's `credential`). Any other envelope first, another version, or a wrong `access_token`, `instance_token` or `credential` SHALL get `ConnectionError` (`expected_hello`, `unsupported_version`, `denied`) and the connection closed; nothing SHALL be sent before `HelloAck`.

**Verified by**: `bus::hello_comes_first_and_names_a_version`, `bus::the_first_thing_a_participant_hears_is_its_hello_ack`.

#### Scenario: Call before Hello
- **WHEN** a WebSocket client sends `Call{method: "pane.list"}` first
- **THEN** it receives `ConnectionError{code: "expected_hello"}` and the connection closes

#### Scenario: Newer version
- **WHEN** a client sends `Hello` with `protocol_version` 2
- **THEN** it receives `ConnectionError` whose message says `unsupported protocol version 2: this server speaks 1`, and the connection closes

#### Scenario: Events before Hello
- **WHEN** a client connects, panes are created meanwhile, and it then says `Hello`
- **THEN** the first envelope it receives is `HelloAck`

### Requirement: Unknown envelopes are ignored and bad ones reported
Both sides SHALL ignore envelope variants and fields they do not know. On a WebSocket, a message whose flag, compression or protobuf does not decode SHALL be answered with `ConnectionError{code: "bad_frame"}` and the connection MUST stay open; on an instance transport it SHALL close the connection.

**Verified by**: `bus::unknown_envelopes_are_ignored_and_bad_ones_reported`, unit test `codec::tests::unknown_envelope_variants_are_ignored`.

#### Scenario: Unknown flag byte
- **WHEN** a greeted WebSocket client sends the binary message `[9, 1, 2]` and then calls `pane.list`
- **THEN** it receives `ConnectionError` with code `bad_frame`, then the reply to `pane.list`

### Requirement: The broker sets who is calling
The broker SHALL set `from` of every forwarded call, event and stream to the sender's identity (`tilrd`, `owner`, `instance:<name>` or `token:<name>`), whatever the sender wrote, and SHALL accept a `Reply`, `Chunk` or `End` only from the connection the call was sent to.

**Verified by**: `bus::from_is_always_the_callers_identity`.

#### Scenario: Forged from
- **WHEN** `t-1` calls `test.from` on `t-2` with `from` set to `owner`
- **THEN** `t-2` sees `from` `instance:t-1`

### Requirement: Calls are routed by target or by method
A `Call` with a `target` SHALL go to that participant; one without SHALL go to a live, non-draining provider of the method, chosen by the method's `select`: in turn in connection order for `ROUND_ROBIN`, the provider that sent `Activate` last (else the last connected) for `LAST_ACTIVE`. Methods in the namespaces `bus`, `pane`, `server`, `fs`, `exec` and `clipboard` SHALL be provided by `tilrd` only; a `Hello` providing one MUST be refused with `ConnectionError{code: "reserved"}`.

**Verified by**: `bus::calls_go_round_robin_over_the_providers`, `bus::last_active_goes_to_the_provider_that_activated_last`, `bus::reserved_namespaces_cannot_be_provided`.

#### Scenario: Two workers
- **WHEN** instances `t-1` and `t-2` provide `test.whoami` and the owner calls it four times without a target
- **THEN** the replies are `t-1`, `t-2`, `t-1`, `t-2`

#### Scenario: Two windows
- **WHEN** participants `w-1` and `w-2` provide `ui.notify` with `LAST_ACTIVE`, `w-2` sends `Activate`, then `w-1` sends `Activate`, and the owner calls `ui.notify`
- **THEN** `w-1` handles it

#### Scenario: Pretending to be the core
- **WHEN** an instance says `Hello` providing `pane.create`
- **THEN** it receives `ConnectionError` with code `reserved` and is restarted with backoff

### Requirement: Calls between two participants keep their order
Calls, and the cancellations of streams, sent by one connection to one provider connection SHALL be delivered in the order they were sent.

**Verified by**: `bus::calls_between_two_participants_keep_their_order`.

#### Scenario: A thousand calls
- **WHEN** `t-1` sends 1000 calls to `test.record` on `t-2` without waiting for replies
- **THEN** `t-2` records them in the order sent

### Requirement: The sender learns why a call failed
A call SHALL be answered with `Error.code` `not_found` when its target or method is unknown, `unavailable` when the target or every provider is not up, is draining or goes away before replying, `timeout` when no reply comes within `min(Call.timeout_ms, call_timeout_ms)`, `denied` when it is not allowed, and `busy` when the caller has `max_in_flight` calls pending or the callee's queue is full; the message SHALL name the participant or method.

**Verified by**: `bus::a_failed_call_says_why`, `bus::a_full_queue_is_answered_busy`.

#### Scenario: Unknown method
- **WHEN** the owner calls `nobody.here`
- **THEN** the reply is `not_found` naming `nobody.here`

#### Scenario: No window open
- **WHEN** no participant provides `ui.notify` and an instance calls it
- **THEN** the reply is `unavailable` with `no provider of ui.notify is connected`

#### Scenario: In-flight limit
- **WHEN** `t-1` sends 300 calls to a method that never replies
- **THEN** 256 stay pending and 44 are answered `busy`

### Requirement: Deadlines and cancellation reach the callee
A forwarded call SHALL carry in `timeout_ms` the time its caller has left; when a call times out, its caller cancels it with `Cancel`, or its caller's connection ends, the broker SHALL send the callee `Cancel{id}` and SHALL drop a later `Reply`, `Chunk` or `End` for it.

**Verified by**: `bus::a_nested_call_gets_the_time_that_is_left_and_a_cancel`.

#### Scenario: Nested deadline
- **WHEN** the owner calls `test.nested` on `t-1` with `timeout_ms` 1000 and `t-1` waits 400 ms before calling `test.deadline` on `t-2`
- **THEN** `t-2` sees `timeout_ms` of at most 600

#### Scenario: Caller gone
- **WHEN** a caller's connection ends while its call to `test.sleep` runs on `t-2`
- **THEN** `t-2` receives `Cancel` with that call's id

### Requirement: Events reach the subscribers connected now
An `Event` SHALL be delivered once to every connection that holds, at the time it is emitted, a subscription whose pattern matches its subject (segments, `*` for one segment, a trailing `>` for one or more) and that is allowed to receive it, in emit order per emitter, with `from` set by the broker. The broker MUST NOT store events: a subscriber that connects or subscribes later SHALL NOT receive earlier events.

**Verified by**: `bus::events_reach_the_subscribers_connected_now`, unit test `events::tests::patterns_match_as_nats_subjects` (crates/tilrd/src/bus/events.rs).

#### Scenario: Wildcard
- **WHEN** `t-2` subscribes to `test.*` and `t-3` to `test.>`, and `t-1` emits `test.a` and `test.a.b`
- **THEN** `t-2` receives `test.a` only and `t-3` receives both

#### Scenario: No history
- **WHEN** `t-1` emits `test.a`, then `t-4` subscribes to `test.*`
- **THEN** `t-4` receives nothing until the next event

### Requirement: A lagging subscriber loses events, not its connection
When the interactive lane of a subscriber's connection lags (a packet held over 250 ms), events for it SHALL be dropped and counted in `tilrd_bus_events_dropped_total`; the connection MUST NOT be closed for it.

**Verified by**: `bus::a_lagging_subscriber_loses_events_not_its_connection`.

#### Scenario: Subscriber stops reading
- **WHEN** a WebSocket subscriber of `test.*` stops reading while `t-1` emits 100000 events, then reads again
- **THEN** it is still connected, it received fewer events than were emitted, and the metric counted the rest

### Requirement: A stream is a call answered in chunks
A call to a method whose `MethodSpec.kind` is `STREAM` SHALL be answered by any number of `Chunk` envelopes followed by one `End` or an error `Reply`, all with the call's id, in order; a `Reply` with a payload to a stream SHALL reach the caller as `bad_reply`. The caller MAY end it with `Cancel`; the provider SHALL then receive `Cancel` and later chunks SHALL be dropped. A stream SHALL be authorized as a call.

**Verified by**: `bus::a_stream_is_a_call_answered_in_chunks`.

#### Scenario: Count to three
- **WHEN** the owner opens `test.count {n: 3}`
- **THEN** it receives chunks `1`, `2`, `3` and then `End`

#### Scenario: Cancelled
- **WHEN** the owner opens `test.count {n: 1000000}` and sends `Cancel` after the first chunk
- **THEN** the test runtime records the cancel and no `End` arrives

### Requirement: Streams are paced by the reader's credit
A provider SHALL send at most as many payload bytes on a stream as the caller granted (`Call.window`, else `HelloAck.stream_window`, 1 MiB; at most 16 MiB) plus what the caller returned with `Credit`; the broker MUST end a stream whose provider overruns its credit with `bad_reply` and send the provider `Cancel`. A stream whose reader does not credit SHALL stall only that stream.

**Verified by**: `bus::streams_are_paced_by_the_readers_credit`, `bus::a_stream_that_overruns_its_credit_is_ended`.

#### Scenario: Slow reader
- **WHEN** the owner opens `test.flood` with `window` 65536 and credits nothing, while it also calls `test.echo`
- **THEN** it receives at most 65536 bytes of chunks, `test.echo` is answered at once, and after a `Credit` of 65536 more chunks arrive

#### Scenario: Overrun
- **WHEN** a provider sends 2 MiB on a stream opened with a 1 MiB window
- **THEN** the caller gets `bad_reply` and the provider receives `Cancel`

### Requirement: Bulk data in the network is bounded per connection
Across all bulk streams to one connection at most `HelloAck.connection_window` (1 MiB) payload bytes SHALL be sent and not yet credited by the reader; further bulk chunks SHALL wait, while interactive traffic to that connection SHALL keep flowing.

**Verified by**: `bus::bulk_data_in_the_network_is_bounded_per_connection`, `files::a_client_answering_only_the_latest_ping_gets_the_whole_file`.

#### Scenario: Two reads, no credit
- **WHEN** a participant opens two `fs.read` streams of 8 MiB each with 4 MiB windows and credits nothing
- **THEN** it receives at most 1 MiB of chunks in all, and its `pane.output` frames still arrive

### Requirement: Interactive traffic goes ahead of bulk traffic
Each connection's outbound queue SHALL have an interactive lane (replies and chunks of `INTERACTIVE` methods, events, credits, cancels) and a bulk lane (chunks and replies of `BULK` methods) bounded at 4 MiB; the bulk lane MUST be written only when the interactive lane is empty. A connection whose interactive lane holds 1024 envelopes when another must be queued SHALL be closed with code 4001 `too slow`.

**Verified by**: `bus::interactive_traffic_goes_ahead_of_bulk_traffic`, unit tests `interactive_packets_go_ahead_of_bulk_ones` and `bulk_waits_for_room_counted_in_bytes` (crates/tilrd/src/outbox.rs).

#### Scenario: Echo during a bulk stream
- **WHEN** a WebSocket participant reads a 64 MiB `test.bulk` stream and calls `test.echo` meanwhile
- **THEN** each echo is answered before the next bulk megabyte reaches it

### Requirement: A slow participant does not hold up the others
Calls, events and streams between participants SHALL be routed while another participant is slow to read or to reply.

**Verified by**: `bus::a_slow_participant_does_not_hold_up_the_others`.

#### Scenario: One instance stops reading
- **WHEN** `t-3` stops reading its connection and the owner calls `test.echo` on `t-1` 1000 times
- **THEN** every call is answered within the usual time

### Requirement: Payloads are bytes the broker does not read
The broker SHALL pass `payload`, `content_type` and `content_encoding` of calls, replies, chunks and events between participants unchanged, byte for byte, including empty and non-UTF-8 payloads. Methods of `tilrd` SHALL take and give the protobuf messages of `crates/tilr-bus/proto/tilr/` (`application/x-protobuf`), and a request with `content_type` `application/json` SHALL be answered in the proto3 JSON mapping.

**Verified by**: `bus::payloads_pass_byte_for_byte`, `bus::core_methods_answer_json_to_json`.

#### Scenario: Binary payload
- **WHEN** the owner calls `test.echo` with 1 MiB of random bytes and `content_type` `application/octet-stream`
- **THEN** the reply has the same bytes and the same `content_type`

#### Scenario: JSON for scripts
- **WHEN** the owner calls `pane.list` with payload `{}` and `content_type` `application/json`
- **THEN** the reply's `content_type` is `application/json` and its payload parses as JSON with a `panes` array

### Requirement: Instances come from bus.toml
The server SHALL read `bus.toml` in its configuration directory at start and on `bus.reload_config`, with only the keys `call_timeout_ms`, `drain_ms`, `[modules.<name>]` (`command`, `cwd`, `env`) and `[instances.<name>]` (`module`, `connect`, `autostart`, `rights`, `env`), and SHALL start every instance whose `autostart` is not false. A missing file SHALL mean no instances.

**Verified by**: `bus::an_instance_from_bus_toml_is_started_and_answers`.

#### Scenario: One instance
- **WHEN** `bus.toml` defines module `t` (the test runtime) and instance `t-1` with `rights.provide = ["test.*"]`, and the server starts
- **THEN** `tilr bus ls` lists `t-1` as `up` and `tilr bus call test.echo hi` prints `hi`

#### Scenario: Not autostarted
- **WHEN** an instance has `autostart = false`
- **THEN** it is listed as `stopped` and no process runs for it until `bus.start`

### Requirement: A bad bus.toml does not stop the server
A `bus.toml` with an unknown key (including the old `calls`, whose message SHALL name `rights`), a wrong type, an instance naming no module, an empty `command`, a duplicate or reserved instance name (`owner`, `tilrd`, `bus…`) or a socket path over the platform limit (104 bytes macOS, 108 Linux) SHALL be logged naming the file and the reason; at start the server SHALL serve panes as usual with no instances, and `bus.reload_config` SHALL answer `bad_config` and change nothing.

**Verified by**: `bus::a_bad_bus_toml_leaves_panes_working`, `bus::reloading_a_bad_bus_toml_changes_nothing`, `bus::a_socket_path_that_is_too_long_is_a_config_error`.

#### Scenario: Misspelt key at start
- **WHEN** `bus.toml` says `[instances.t-1] modul = "t"` and the server starts
- **THEN** the log names `bus.toml` and `modul`, panes can be created, and `tilr bus ls` lists nothing

#### Scenario: The old key
- **WHEN** `bus.toml` says `[instances.t-1] calls = ["test.*"]`
- **THEN** the reason says `calls` was replaced by `[instances.t-1.rights] call`

#### Scenario: Misspelt key on reload
- **WHEN** instance `t-1` runs, `bus.toml` is edited to contain an unknown key, and `tilr bus reload-config` runs
- **THEN** it exits 1 with `bad_config: ` and `t-1` keeps running with the same pid

### Requirement: A spawned instance inherits its connection
An instance with `connect = "fd"` (the default) SHALL be started in its own session with stdin `/dev/null`, its bus connection as descriptor 3, and the environment of the server (without the pane variables) plus the module's and then the instance's `env`, plus `TILR_BUS_FD=3`, `TILR_INSTANCE=<name>` and `TILR_CONFIG_DIR=<config dir>`; no other descriptor of the server SHALL be open in it.

**Verified by**: `bus::an_instance_gets_its_connection_and_names`.

#### Scenario: Environment and descriptors
- **WHEN** the test runtime reports its environment and open descriptors through `test.env`
- **THEN** it shows `TILR_BUS_FD=3`, `TILR_INSTANCE=t-1`, the instance's `env` over the module's, and descriptors 0, 1, 2 and 3 only

### Requirement: The instance socket takes one token once
An instance with `connect = "socket"` SHALL get `TILR_BUS_SOCK` (`<runtime dir>/bus/<name>.sock`, mode 0600, in a 0700 directory) and `TILR_BUS_TOKEN` (32 hex characters, new for each process start); the server SHALL bind the first `Hello` whose `instance_token` is that token to the instance and MUST refuse any other connection to that socket, closing it; a connection that sends no valid `Hello` within 10 seconds SHALL be closed.

**Verified by**: `bus::a_socket_instance_connects_once_with_its_token`.

#### Scenario: Token reused
- **WHEN** a socket instance is up and another process connects to its socket with the same token
- **THEN** that connection is closed without `HelloAck` and the instance stays `up`

### Requirement: A crashed or disconnected instance is restarted with backoff
An instance whose process exits on its own, whose connection ends while its process lives, or that has not said `Hello` 30 seconds after it was started SHALL have its process group killed and be started again after a pause of 200 ms doubling up to 5 s, reset after it has run 10 s; `bus.instances` SHALL count the restarts.

**Verified by**: `bus::a_crashed_instance_is_restarted_with_backoff`, `bus::an_instance_that_loses_its_connection_is_restarted`, `bus::an_instance_that_never_says_hello_is_restarted`.

#### Scenario: Crash loop
- **WHEN** the test runtime exits at once three times
- **THEN** the starts are about 200, 400 and 800 ms apart and `restarts` is 3

#### Scenario: Connection closed
- **WHEN** the test runtime closes descriptor 3 but keeps running (`test.close_bus`)
- **THEN** its process is killed and a new process says `Hello`

### Requirement: Stopping, starting and reloading instances
`bus.stop {instance, drain_ms?}` SHALL drain the instance up to `drain_ms`, send SIGTERM to its group and SIGKILL after 3 s, and keep it `stopped` until `bus.start {instance}`, which SHALL answer once it said `Hello`. `bus.reload {instance | module | all}` SHALL restart them one at a time, each drained and waited for until `Hello`, answering old and new pids or `spawn_failed`, `start_failed`, `start_timeout`. A caller acting on itself SHALL get its reply first.

**Verified by**: `bus::an_instance_is_stopped_and_started`, `bus::a_module_is_reloaded_one_instance_at_a_time`.

#### Scenario: No gap for a method
- **WHEN** `t-1` and `t-2` provide `test.echo`, the owner calls it in a loop and `tilr bus reload --module t` runs
- **THEN** no call of the loop fails and both instances have new pids

#### Scenario: A caller reloads itself
- **WHEN** `t-1` calls `bus.reload {"instance": "t-1"}`
- **THEN** it receives the reply before its process is stopped

### Requirement: Reloading bus.toml applies the difference
`bus.reload_config` SHALL read `bus.toml` again, stop removed instances, start added ones, reload instances whose module or entry changed (including `rights`), keep the others running with their pids, keep manually stopped instances stopped, and apply `call_timeout_ms` to new calls.

**Verified by**: `bus::reloading_bus_toml_applies_the_difference`.

#### Scenario: Add, change, remove
- **WHEN** `t-2` is added, `t-1`'s `env` changes and `t-3` is removed, then `tilr bus reload-config` runs
- **THEN** `t-2` is up, `t-1` has a new pid, `t-3` no longer runs, and an unchanged `t-4` keeps its pid

### Requirement: Listing participants and methods
`bus.instances` SHALL list every configured instance with its name, module, transport, state (`down`, `starting`, `up`, `draining`, `stopped`), pid, restarts and provided methods, and the outside participants connected now with their identity and `client_name`; `bus.methods` SHALL list each method with its kind, lane and the live, non-draining providers.

**Verified by**: `bus::participants_and_methods_are_listed`.

#### Scenario: Draining is not offered
- **WHEN** `t-1` is being drained by `bus.reload` and `bus.methods` is called
- **THEN** `t-1` is not listed as a provider, and `tilrd` is listed for `pane.output` with kind `STREAM` and lane `INTERACTIVE`

### Requirement: An instance's output goes to the server log
Each line an instance writes to stdout or stderr SHALL appear in the server's log as `bus <name>: <line>`.

**Verified by**: `bus::an_instances_output_is_in_the_server_log`.

#### Scenario: A line on stderr
- **WHEN** the test runtime writes `hello log` to stderr
- **THEN** the server log has a line ending with `bus t-1: hello log`

### Requirement: Instances survive a tilrd reload
On a reload the server SHALL hand its instances, their connections, tokens and subscriptions to the successor: the instances' pids stay the same, they stay `up`, keep their subscriptions and are routable after the reload; calls in flight at the exec SHALL be answered `unavailable` and their callees sent `Cancel`, and open streams SHALL end with `unavailable`.

**Verified by**: `reload::bus_instances_survive_a_reload`.

#### Scenario: Same pid after reload
- **WHEN** `t-1` is up and subscribed to `pane.*`, the server gets SIGUSR1, and the owner reconnects and creates a pane
- **THEN** `tilr bus ls` shows `t-1` `up` with the same pid and `restarts` unchanged, `t-1` receives `pane.created`, and `tilr bus call test.echo x` prints `x`

### Requirement: tilr bus
`tilr bus call METHOD [PAYLOAD | - | --file PATH] [--target NAME] [--timeout DURATION] [--content-type TYPE] [--credential TOKEN]` SHALL make the call (`content_type` `application/json` by default) and write the reply's payload to stdout unchanged and exit 0, or write `<code>: <message>` to stderr and exit 1; `tilr bus ls`, `start`, `stop`, `reload` and `reload-config` SHALL call the matching `bus.*` method and print its result.

**Verified by**: `client::tilr_bus_call_prints_the_reply`.

#### Scenario: Echo from stdin
- **WHEN** the user runs `printf 'a\0b' | tilr bus call test.echo -`
- **THEN** stdout is the 3 bytes `a`, NUL, `b` and the exit status is 0

#### Scenario: Missing method
- **WHEN** the user runs `tilr bus call nobody.here`
- **THEN** stderr starts with `not_found: ` and the exit status is 1

### Requirement: tilr bus sub
`tilr bus sub PATTERN` SHALL subscribe to `PATTERN` and print one line per event (subject, a space, the payload as JSON for the core's events, else base64) until interrupted.

**Verified by**: `client::tilr_bus_sub_prints_events`.

#### Scenario: Watching events
- **WHEN** the user runs `tilr bus sub 'pane.*'` and a pane is created
- **THEN** a line starting with `pane.created ` is printed
