# Design: Credits, lanes, limits and deadlines on the bus

Shared design: `openspec/design/unified-bus.md` §B5 (deadlines, errors, limits), §B6 (slow
subscriber), §B7 (credits, lanes, latest-wins), §B8 (bounded queues, no global lock). This
change implements them behind the contract of `bus-broker-core`'s design; it changes no type of
that contract.

## Decisions

- **Tests without instances.** As in `bus-broker-core`, scenarios naming `t-N` run the test
  runtime as WebSocket participants (`bus-instances` lands in parallel). Scenarios naming
  `fs.read` or `pane.output` are run with `test.bulk` and `test.flood`; the core streams are
  checked by their own changes.
- **Second editor of the broker.** After `bus-broker-core`, this change is the one that edits
  `bus/broker.rs` and `bus/transport.rs` (then `bus-reload`, after it merged; nobody else):
  `busy` and `max_in_flight`, deadlines, credits, events dropped for a laggard, the connection
  window and the memory bound of the bulk relay go there.
- **Credit is what the reader consumed.** The SDK side and `tilr-bus` already return credit as
  chunks are consumed (`bus-wire`); the broker relays and enforces it.

## Reload and reconnect

No new state. A reload ends the connections (and so their streams and subscriptions) as
today; `bus-reload` adds the drain before the exec (B13).

## Backpressure

This change is the backpressure of the bus: per-stream and per-connection credit, 1024
interactive envelopes and 4 MiB bulk per connection, 256 pending calls per caller, `busy` instead
of growth, events dropped for a lagging subscriber (B7, B8).

## Deviations in the implementation

What the code chose where the shared design leaves room or says otherwise; the code is the
contract. No signature of `bus-broker-core`'s contract changed. Added, all additive:
`BrokerHandle::set_call_timeout` (for `bus-instances`, from `call_timeout_ms`),
`know_methods` (methods an instance of bus.toml will provide), `send_credential_update`
(a `CredentialUpdate` to one connection, for instance token renewal and rotation),
`watch_connection` (a `watch` of a connection's `ConnState`, to be told when it ends),
`StreamOut::window` and `StreamOut::caught_up_for(bytes)`; `transport::Out` gained
`try_encode`, `bulk_queued`, `bulk_room` and the constants `INTERACTIVE_BYTES`,
`OPTIONAL_BYTES`, `BULK_ENVELOPES`, `BULK_BYTES`, `MAX_PROVIDES`; `broker::RELAY_BUDGET`
and `CHUNK_COST`.

- **One mutex, not a split per connection.** B8 asks for no global lock. The broker keeps
  one mutex over its table, held only for bookkeeping: never across an await, a call of
  the `Authorizer` (`on_call`, `on_subscribe`, `on_event`, `deliver`,
  `on_credential_update` and `greet` call it with the lock released), or the encoding
  of a call, a reply, a chunk, an `End` or an event: those are encoded before and
  queued under it with a non-blocking push. Only the broker's own small errors
  (`timeout`, `unavailable`, `bad_reply`, `too_large`) are encoded under it, so that
  they stay behind the chunks of their stream. Splitting it per connection would need
  a lock order between a caller's and a callee's entries on every call, credit and end;
  the goal of B8, that a slow participant delays only its own calls, holds without it
  (`bus::a_slow_participant_does_not_hold_up_the_others`).
- **The timer wheel is an ordered set** of `(deadline, caller, id, hop)` served by one
  task, armed when a call is routed and removed with its answer; an entry of a call that
  ended otherwise is ignored when it fires.
- **One answer per call.** A call is answered by whoever ends its route first (the
  reply, the deadline, a cancel, a connection's end, a refusal when it could not be
  queued); the others find no route and say nothing. A `Cancel` goes to the callee only
  once its `Call` is queued: a call that ended in between gets its `Cancel` after it.
- **`not_found` or `unavailable`.** The broker cannot know every method a participant
  might provide. A call of a method without a live provider is `unavailable` (`no
  provider of <method> is connected`) if some participant provides it now (all
  draining), an instance of bus.toml will (`know_methods`, kept whole), or it is a
  method the spec gives to windows and plugins (`ui.notify`, `ui.pick_file`) or to
  clients (`host.open`, `ssh_agent.*`); any other is `not_found`. A name only a
  participant that is gone provided is forgotten with it, so connections cannot fill
  the broker with names. A target that is not connected stays `unavailable` (instances
  come with `bus-instances`). `bus::calls_go_round_robin_over_the_providers` now expects
  `not_found` for `nobody.here`.
- **A stream's deadline is its own.** "Streams have no default deadline": a stream gets one
  only from its `timeout_ms`, not capped by `call_timeout_ms` (a long download is not cut
  at 10 s); a unary call over the wire gets `min(timeout_ms, call_timeout_ms)`, 0 the
  default. `BrokerHandle::call` (tilrd calling a participant) honours the timeout its
  `CallOptions` give as given; without one it gets `call_timeout_ms` (none if 0).
- **"The callee's queue is full"** is 768 of its 1024 interactive envelopes, or 24 of its
  32 MiB (`transport::OPTIONAL_ROOM`, `OPTIONAL_BYTES`): a call is then answered `busy`
  and an event for it dropped, without closing it; the rest of the lane is room for what
  must be queued (replies, cancels, credits). A lane full of those (1024 envelopes or
  32 MiB unwritten, `INTERACTIVE_BYTES`) still closes 4001: the interactive lane is
  bounded in bytes too, so a reader that stops reading relayed interactive streams of
  large chunks (credited up to 16 MiB each) is closed, not given 1024 × 16 MiB.
- **A subscriber lags** when its interactive lane has held a packet over 250 ms or holds
  768: an event that would come into a lane filling within 250 ms is dropped too, so a
  burst does not close the subscriber 4001.
- **The credit unit is the bytes of `Chunk.payload` as carried in the envelope**, on all
  three sides (the producer, the broker, the reader; `tilr-bus` counts the same): a
  payload whose `content_encoding` is set (`zstd`) counts compressed, and the
  compression of WebSocket messages never counts.
- **The memory of the relay.** A relayed bulk stream's provider is given
  `min(Call.window, 1 MiB)` as its window, and the broker grants it credit as its chunks
  go on to the reader, never beyond the reader's credit. What the broker holds and has
  granted of the relayed bulk streams one connection reads is at most `RELAY_BUDGET`
  (16 MiB): `sum(granted + held)` over them, each held chunk counted as at least
  `CHUNK_COST` (1 KiB). A stream that would pass it is not opened (`busy`); a grant
  waits for room. A finished stream whose end waits behind held chunks still counts
  there and toward `max_in_flight`. So per connection the relay holds at most 16 MiB of
  payload in at most 16384 chunks, whatever the reader does. Credit stays in payload
  bytes: only chunks far smaller than 1 KiB can fill the budget without filling the
  credit; the stream whose chunk would pass it is ended `bad_reply` (its provider gets
  `Cancel`), as HTTP/2 servers end a flood of tiny or empty DATA frames. Chunks of
  relayed interactive streams are queued at once (the interactive lane's bounds hold
  them); their credits are relayed as they are.
- **Empty chunks.** An empty chunk is a chunk: it is relayed and delivered like any
  other (the client library can write one, so refusing it would break every SDK that
  does). It uses no credit; held for a reader it costs `CHUNK_COST` against the relay
  budget like any tiny chunk, so a flood of them ends its stream `bad_reply` once the
  budget is passed (the provider gets `Cancel`).
- **The bulk lane** holds at most 1024 envelopes and 4 MiB unwritten
  (`BULK_ENVELOPES`, `BULK_BYTES`; a chunk into an empty lane always goes): the
  connection window alone does not bound it, since the window of a stream that ended
  goes back before its chunks are written. The writer has the broker queue what waited
  once the lane is half empty.
- **A chunk the broker cannot relay** (over `MAX_FRAME` once framed for its reader, the
  reader's id being longer than the hop) ends its stream with `too_large` and `Cancel`
  to the provider, as one larger than the reader's frame limit would.
- **The connection window counts open streams.** When a bulk stream ends for its reader
  (`End`, an error, `Cancel`), what it sent and the reader did not credit yet goes back to
  the window, so a reader that drops a finished stream without reading it does not close
  its window for good; a `Credit` for a stream that ended is ignored.
- **Native producers.** `StreamOut::send` waits for credit, the connection window and room
  in the bulk lane; the chunk is checked, taken and queued under one lock, so no chunk
  of a stream that timed out, was cancelled or ended (or whose id a new call reuses)
  is queued, and none comes after its end. `try_send` is `Full` when it would wait. A
  chunk larger than the stream's window (and than the credit it holds) can never be
  sent: the stream ends with `too_large` and `send` and `try_send` answer `Closed`, never
  `Full`, so a latest-wins producer does not loop; `StreamOut::window` tells it what to
  split to. `is_lagging` is also true with no credit left (or a spent connection window
  or a full bulk lane for a bulk stream); `caught_up` waits for at least a byte of
  credit (and for a bulk stream the window and the lane), `caught_up_for(n)` for room
  for a chunk of `n`, so a latest-wins producer does not spin on a stream it cannot
  write. The end (`end`, `fail`, a drop) never waits for credit.
- **The tests of `bus-broker-core` that relied on unbounded relaying** were fitted to the
  bounds: `bus::calls_between_two_participants_keep_their_order` sends its 1000 calls in
  batches of 250 (256 pending per caller), `bus::the_end_of_a_relayed_bulk_stream_comes_after_its_chunks`
  sends 800 KiB (within the windows its caller never credits;
  `bus::the_end_of_a_relayed_bulk_stream_waits_behind_held_chunks` covers an end behind
  held chunks), and the unit test `the_end_of_a_bulk_stream_comes_after_its_chunks`
  credits what it reads.
- **Metric.** `tilrd_bus_events_dropped_total{subject}` counts per subject for the first
  1000 subjects and the rest as `other`, so a flood of subjects does not grow the scrape.
- **A `Hello` provides at most 1024 methods** (`MAX_PROVIDES`): more is refused with
  `ConnectionError{code: "bad_request"}`, the code a `Hello` with a bad method name gets,
  so one participant cannot grow the routing table without bound.
- **A subscriber that loses events is not told** on the wire: the broker counts them in
  `tilrd_bus_events_dropped_total` only; the protocol has no envelope for it.
