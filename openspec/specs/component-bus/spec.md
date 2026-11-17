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
A call SHALL be answered `not_found` when its target does not provide the method or the method is not known, `unavailable` when its target is not connected or a known method has no live, non-draining provider, or the provider goes away before replying, `timeout` at its deadline and `denied` when not allowed; the message SHALL name the participant or method. A method is known while a connection provides it, or when the configuration names it, it is a client method, `ui.notify` or `ui.pick_file`.

**Verified by**: `bus::a_failed_call_says_why`, unit test `a_draining_provider_is_unavailable` (crates/tilrd/src/bus/broker.rs).

#### Scenario: Unknown method
- **WHEN** the owner calls `nobody.here`
- **THEN** the reply is `not_found` naming `nobody.here`

#### Scenario: No window open
- **WHEN** no participant provides `ui.notify` and an instance calls it
- **THEN** the reply is `unavailable` with `no provider of ui.notify is connected`

### Requirement: A call is answered busy when a queue is full
A call SHALL be answered `busy` when its caller has 256 (`max_in_flight`) calls and streams pending, counting finished streams whose end still waits behind held chunks; when the callee's interactive lane holds 768 envelopes or 24 MiB unwritten; or when it opens a relayed bulk stream that would pass the reader's relay budget (16 MiB). A callee is not closed for being busy.

**Verified by**: `bus::a_full_queue_is_answered_busy`, `bus::a_reader_that_never_credits_cannot_grow_the_relay`.

#### Scenario: In-flight limit
- **WHEN** `t-1` sends 300 calls to a method that never replies
- **THEN** 256 stay pending and 44 are answered `busy`

#### Scenario: A callee that stops reading
- **WHEN** six callers each send 256 calls of 4 KiB to a provider that stops reading
- **THEN** the calls past 768 queued are answered `busy` naming the method, and the provider stays connected

### Requirement: Deadlines and cancellation reach the callee
A call's deadline SHALL be `min(Call.timeout_ms, call_timeout_ms)` (10 s unless configured; `timeout_ms` 0 takes `call_timeout_ms`, and 0 there means none); a stream's SHALL be its own `timeout_ms` only, none if 0. A forwarded call SHALL carry the time left in `timeout_ms`; at the deadline, on the caller's `Cancel` or when the caller's connection ends, the callee SHALL get `Cancel{id}` and a later `Reply`, `Chunk` or `End` SHALL be dropped.

**Verified by**: `bus::a_nested_call_gets_the_time_that_is_left_and_a_cancel`, `bus::late_answers_after_a_timeout_or_a_cancel_are_dropped`, unit test `a_native_call_past_its_deadline_is_answered_timeout_and_cancelled` (crates/tilrd/src/bus/broker.rs).

#### Scenario: Nested deadline
- **WHEN** the owner calls `test.nested` on `t-1` with `timeout_ms` 1000 and `t-1` waits 400 ms before calling `test.deadline` on `t-2`
- **THEN** `t-2` sees `timeout_ms` of at most 600

#### Scenario: Caller gone
- **WHEN** a caller's connection ends while its call to `test.sleep` runs on `t-2`
- **THEN** `t-2` receives `Cancel` with that call's id

### Requirement: A lagging subscriber loses events, not its connection
An event SHALL be dropped for a subscriber whose interactive lane has held a packet over 250 ms, or holds 768 envelopes or 24 MiB unwritten; dropped events SHALL be counted in `tilrd_bus_events_dropped_total`, and the connection MUST NOT be closed for it.

**Verified by**: `bus::a_lagging_subscriber_loses_events_not_its_connection`.

#### Scenario: Subscriber stops reading
- **WHEN** a WebSocket subscriber of `test.*` stops reading while `t-1` emits 100000 events, then reads again
- **THEN** it is still connected, it received fewer events than were emitted, and the metric counted the rest

### Requirement: Streams are paced by the reader's credit
A provider SHALL send at most as many bytes of `Chunk.payload`, counted as carried in the envelope (compressed when `content_encoding` is set), as the caller granted (`Call.window`, else `HelloAck.stream_window`, 1 MiB; at most 16 MiB) plus what it returned with `Credit`; the broker MUST end a stream whose provider overruns its credit with `bad_reply` and send the provider `Cancel`. A stream whose reader does not credit SHALL stall only that stream.

**Verified by**: `bus::streams_are_paced_by_the_readers_credit`, `bus::a_stream_that_overruns_its_credit_is_ended`.

#### Scenario: Slow reader
- **WHEN** the owner opens `test.flood` with `window` 65536 and credits nothing, while it also calls `test.echo`
- **THEN** it receives at most 65536 bytes of chunks, `test.echo` is answered at once, and after a `Credit` of 65536 more chunks arrive

#### Scenario: Overrun
- **WHEN** a provider sends 2 MiB on a stream opened with a 1 MiB window
- **THEN** the caller gets `bad_reply` and the provider receives `Cancel`

### Requirement: The end of a stream comes after its chunks
The end of a stream (`End`, an error, `unavailable`, `timeout`) SHALL reach its reader after every chunk of it the broker queued or holds, on the stream's lane, and SHALL NOT wait for credit.

**Verified by**: `bus::the_end_of_a_relayed_bulk_stream_waits_behind_held_chunks`, unit test `the_end_of_a_native_stream_needs_no_credit` (crates/tilrd/src/bus/broker.rs).

#### Scenario: Ended while chunks are held
- **WHEN** a provider sends 2 MiB on a bulk stream to a reader that reads none of it yet, and then `End`
- **THEN** the reader receives every chunk, then `End`

### Requirement: Bulk data in the network is bounded per connection
Across all bulk streams to one connection at most `HelloAck.connection_window` (1 MiB) payload bytes SHALL be sent and not yet credited by the reader; further bulk chunks SHALL wait, while interactive traffic to that connection SHALL keep flowing. When a bulk stream ends, what it sent and the reader did not credit SHALL go back to the window.

**Verified by**: `bus::bulk_data_in_the_network_is_bounded_per_connection`, `bus::a_bulk_stream_that_ends_uncredited_gives_the_window_back`, unit tests `a_native_bulk_stream_waits_for_the_connection_window`, `caught_up_waits_for_the_connection_window` (crates/tilrd/src/bus/broker.rs).

#### Scenario: Two reads, no credit
- **WHEN** a participant opens two `fs.read` streams of 8 MiB each with 4 MiB windows and credits nothing
- **THEN** it receives at most 1 MiB of chunks in all, and its `pane.output` frames still arrive

### Requirement: The relay is bounded per reader
The provider of a relayed bulk stream SHALL be given `min(Call.window, 1 MiB)` and then credit as its chunks go on to the reader, never past the reader's credit. What the broker has granted and holds of the relayed bulk streams one connection reads SHALL stay within 16 MiB, each held chunk counted as at least 1 KiB (an empty one too); a chunk that would pass it ends its stream `bad_reply`, and a chunk that cannot be relayed ends it `too_large`, with `Cancel` to the provider.

**Verified by**: `bus::a_reader_that_never_credits_cannot_grow_the_relay`, `bus::an_empty_chunk_is_relayed_and_a_flood_of_tiny_ones_ends_the_stream`, `bus::a_chunk_too_large_to_relay_ends_its_stream`.

#### Scenario: A reader that never credits
- **WHEN** a reader opens relayed bulk streams of 1 MiB one after another and credits none
- **THEN** 16 to 18 are opened, the next is answered `busy`, and when the reader credits at last it receives all of them

#### Scenario: A flood of empty chunks
- **WHEN** a provider sends empty chunks on a bulk stream to a reader that does not read
- **THEN** an empty chunk is relayed like any other, and the stream ends `bad_reply` before 65536 of them are taken

### Requirement: Interactive traffic goes ahead of bulk traffic
Each connection's outbound queue SHALL have an interactive lane (replies and chunks of `INTERACTIVE` methods, events, credits, cancels) of at most 1024 envelopes or 32 MiB unwritten, and a bulk lane (chunks and replies of `BULK` methods) of at most 1024 envelopes or 4 MiB; the bulk lane MUST be written only when the interactive lane is empty. A connection whose interactive lane is at either bound when another envelope must be queued SHALL be closed with code 4001 `too slow`.

**Verified by**: `bus::interactive_traffic_goes_ahead_of_bulk_traffic`, `bus::a_reader_that_stops_reading_large_interactive_chunks_is_closed_too_slow`, unit test `interactive_packets_go_ahead_of_bulk_ones` (crates/tilrd/src/outbox.rs).

#### Scenario: Echo during a bulk stream
- **WHEN** a WebSocket participant reads a 64 MiB `test.bulk` stream and calls `test.echo` meanwhile
- **THEN** each echo is answered before the next bulk megabyte reaches it

#### Scenario: A reader of large interactive chunks stops reading
- **WHEN** a reader opens three interactive streams with 16 MiB windows and reads nothing while their providers send 48 MiB in chunks of 1 MiB
- **THEN** it is closed with 4001 before all of it reached it

### Requirement: A Hello provides at most 1024 methods
A `Hello` that provides more than 1024 methods SHALL be refused with `ConnectionError{code: "bad_request"}` naming how many it provides, and the connection closed; one that provides 1024 SHALL be greeted.

**Verified by**: `bus::a_hello_provides_at_most_1024_methods`.

#### Scenario: Too many methods
- **WHEN** a participant says `Hello` providing 1025 methods
- **THEN** it receives `ConnectionError` with code `bad_request` whose message says `1025 methods`

### Requirement: A slow participant does not hold up the others
Calls, events and streams between participants SHALL be routed while another participant is slow to read or to reply.

**Verified by**: `bus::a_slow_participant_does_not_hold_up_the_others`.

#### Scenario: One instance stops reading
- **WHEN** `t-3` stops reading its connection and the owner calls `test.echo` on `t-1` 1000 times
- **THEN** every call is answered within the usual time
