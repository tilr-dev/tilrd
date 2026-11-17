## ADDED Requirements

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
An `Event` SHALL be delivered once to every connection that holds, at the time it is emitted, a subscription whose pattern matches its subject (segments, `*` for one segment, a trailing `>` for one or more) and that is allowed to receive it, in emit order per emitter, with `from` set by the broker. The broker MUST NOT store events: a subscriber that connects or subscribes later SHALL NOT receive earlier events.

**Verified by**: `bus::events_reach_the_subscribers_connected_now`, unit test `events::tests::patterns_match_as_nats_subjects` (crates/tilrd/src/bus/events.rs).

#### Scenario: Wildcard
- **WHEN** `t-2` subscribes to `test.*` and `t-3` to `test.>`, and `t-1` emits `test.a` and `test.a.b`
- **THEN** `t-2` receives `test.a` only and `t-3` receives both

#### Scenario: No history
- **WHEN** `t-1` emits `test.a`, then `t-4` subscribes to `test.*`
- **THEN** `t-4` receives nothing until the next event

### Requirement: A stream is a call answered in chunks
A call to a method whose `MethodSpec.kind` is `STREAM` SHALL be answered by any number of `Chunk` envelopes followed by one `End` or an error `Reply`, all with the call's id, in order; a `Reply` with a payload to a stream SHALL reach the caller as `bad_reply`. The caller MAY end it with `Cancel`; the provider SHALL then receive `Cancel` and later chunks SHALL be dropped. A stream SHALL be authorized as a call.

**Verified by**: `bus::a_stream_is_a_call_answered_in_chunks`.

#### Scenario: Count to three
- **WHEN** the owner opens `test.count {n: 3}`
- **THEN** it receives chunks `1`, `2`, `3` and then `End`

#### Scenario: Cancelled
- **WHEN** the owner opens `test.count {n: 1000000}` and sends `Cancel` after the first chunk
- **THEN** the test runtime records the cancel and no `End` arrives
