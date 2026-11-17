# The bus broker

## Purpose
`tilrd` serves the bus `tilr.bus.v1` beside `tilr.v1`: participants say Hello, the broker
sets `from`, routes calls, relays streams and events, and is itself the participant `tilrd`.

Verified by `crates/tilrd/tests/bus.rs` and the unit tests of `crates/tilrd/src/bus/`.

## Requirements

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

### Requirement: Events reach the subscribers connected now
An `Event` SHALL be delivered at most once to every connection holding, when it is emitted, an allowed subscription whose pattern matches its subject (segments, `*` for one, a trailing `>` for one or more), in emit order per emitter, with `from` set by the broker; a subscriber that falls behind SHALL lose events, counted, and keep its connection. The broker MUST NOT store events: a later subscriber SHALL NOT receive earlier events.

**Verified by**: `bus::events_reach_the_subscribers_connected_now`, `bus::a_lagging_subscriber_loses_events_not_its_connection`, unit test `events::tests::patterns_match_as_nats_subjects` (crates/tilrd/src/bus/events.rs).

#### Scenario: Wildcard
- **WHEN** `t-2` subscribes to `test.*` and `t-3` to `test.>`, and `t-1` emits `test.a` and `test.a.b`
- **THEN** `t-2` receives `test.a` only and `t-3` receives both

#### Scenario: No history
- **WHEN** `t-1` emits `test.a`, then `t-4` subscribes to `test.*`
- **THEN** `t-4` receives nothing until the next event

#### Scenario: A subscriber that falls behind
- **WHEN** a WebSocket subscriber of `test.*` stops reading while `t-1` emits 100000 events, then reads again
- **THEN** it is still connected, it received fewer events than were emitted, and `tilrd_bus_events_dropped_total` counted the rest

### Requirement: A stream is a call answered in chunks
A call to a method whose `MethodSpec.kind` is `STREAM` SHALL be answered by any number of `Chunk` envelopes followed by one `End` or an error `Reply`, all with the call's id, in order; a `Reply` with a payload to a stream SHALL reach the caller as `bad_reply`. The caller MAY end it with `Cancel`; the provider SHALL then receive `Cancel` and later chunks SHALL be dropped. A stream SHALL be authorized as a call.

**Verified by**: `bus::a_stream_is_a_call_answered_in_chunks`.

#### Scenario: Count to three
- **WHEN** the owner opens `test.count {n: 3}`
- **THEN** it receives chunks `1`, `2`, `3` and then `End`

#### Scenario: Cancelled
- **WHEN** the owner opens `test.count {n: 1000000}` and sends `Cancel` after the first chunk
- **THEN** the test runtime records the cancel and no `End` arrives

### Requirement: The sender learns why a call failed
A call SHALL be answered with `Error.code` `not_found` when its target or method is unknown, `unavailable` when the target or every provider is not up, is draining or goes away before replying, `timeout` when no reply comes within `min(Call.timeout_ms, call_timeout_ms)`, `denied` when it is not allowed, and `busy` when the caller has `max_in_flight` calls pending or the callee's queue is full; the message SHALL name the participant or method.

**Verified by**: `bus::a_failed_call_says_why`, `bus::a_full_queue_is_answered_busy`, unit test `a_draining_provider_is_unavailable` (crates/tilrd/src/bus/broker.rs).

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

**Verified by**: `bus::a_nested_call_gets_the_time_that_is_left_and_a_cancel`, `bus::late_answers_after_a_timeout_or_a_cancel_are_dropped`.

#### Scenario: Nested deadline
- **WHEN** the owner calls `test.nested` on `t-1` with `timeout_ms` 1000 and `t-1` waits 400 ms before calling `test.deadline` on `t-2`
- **THEN** `t-2` sees `timeout_ms` of at most 600

#### Scenario: Caller gone
- **WHEN** a caller's connection ends while its call to `test.sleep` runs on `t-2`
- **THEN** `t-2` receives `Cancel` with that call's id

### Requirement: A lagging subscriber loses events, not its connection
When the interactive lane of a subscriber's connection lags (a packet held over 250 ms), events for it SHALL be dropped and counted in `tilrd_bus_events_dropped_total`; the connection MUST NOT be closed for it.

**Verified by**: `bus::a_lagging_subscriber_loses_events_not_its_connection`.

#### Scenario: Subscriber stops reading
- **WHEN** a WebSocket subscriber of `test.*` stops reading while `t-1` emits 100000 events, then reads again
- **THEN** it is still connected, it received fewer events than were emitted, and the metric counted the rest

### Requirement: Streams are paced by the reader's credit
A provider SHALL send at most as many payload bytes on a stream as the caller granted (`Call.window`, else `HelloAck.stream_window`, 1 MiB; at most 16 MiB) plus what the caller returned with `Credit`; the broker MUST end a stream whose provider overruns its credit with `bad_reply` and send the provider `Cancel`. A stream whose reader does not credit SHALL stall only that stream.

**Verified by**: `bus::streams_are_paced_by_the_readers_credit`, `bus::a_stream_that_overruns_its_credit_is_ended`, `bus::an_empty_chunk_is_relayed_and_a_flood_of_tiny_ones_ends_the_stream`.

#### Scenario: Slow reader
- **WHEN** the owner opens `test.flood` with `window` 65536 and credits nothing, while it also calls `test.echo`
- **THEN** it receives at most 65536 bytes of chunks, `test.echo` is answered at once, and after a `Credit` of 65536 more chunks arrive

#### Scenario: Overrun
- **WHEN** a provider sends 2 MiB on a stream opened with a 1 MiB window
- **THEN** the caller gets `bad_reply` and the provider receives `Cancel`

### Requirement: Bulk data in the network is bounded per connection
Across all bulk streams to one connection at most `HelloAck.connection_window` (1 MiB) payload bytes SHALL be sent and not yet credited by the reader; further bulk chunks SHALL wait, while interactive traffic to that connection SHALL keep flowing.

**Verified by**: `bus::bulk_data_in_the_network_is_bounded_per_connection`, `bus::a_bulk_stream_that_ends_uncredited_gives_the_window_back`, `bus::a_reader_that_never_credits_cannot_grow_the_relay`, unit test `a_native_bulk_stream_waits_for_the_connection_window` (crates/tilrd/src/bus/broker.rs).

#### Scenario: Two reads, no credit
- **WHEN** a participant opens two `fs.read` streams of 8 MiB each with 4 MiB windows and credits nothing
- **THEN** it receives at most 1 MiB of chunks in all, and its `pane.output` frames still arrive

### Requirement: Interactive traffic goes ahead of bulk traffic
Each connection's outbound queue SHALL have an interactive lane (replies and chunks of `INTERACTIVE` methods, events, credits, cancels) and a bulk lane (chunks and replies of `BULK` methods) bounded at 4 MiB; the bulk lane MUST be written only when the interactive lane is empty. A connection whose interactive lane holds 1024 envelopes when another must be queued SHALL be closed with code 4001 `too slow`.

**Verified by**: `bus::interactive_traffic_goes_ahead_of_bulk_traffic`, `bus::a_reader_that_stops_reading_large_interactive_chunks_is_closed_too_slow`, unit test `interactive_packets_go_ahead_of_bulk_ones` (crates/tilrd/src/outbox.rs).

#### Scenario: Echo during a bulk stream
- **WHEN** a WebSocket participant reads a 64 MiB `test.bulk` stream and calls `test.echo` meanwhile
- **THEN** each echo is answered before the next bulk megabyte reaches it

### Requirement: A slow participant does not hold up the others
Calls, events and streams between participants SHALL be routed while another participant is slow to read or to reply.

**Verified by**: `bus::a_slow_participant_does_not_hold_up_the_others`.

#### Scenario: One instance stops reading
- **WHEN** `t-3` stops reading its connection and the owner calls `test.echo` on `t-1` 1000 times
- **THEN** every call is answered within the usual time
