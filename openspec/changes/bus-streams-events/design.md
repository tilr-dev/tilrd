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
- **Credit is what the reader consumed.** The SDK side and `tilr-bus` already return credit as
  chunks are consumed (`bus-wire`); the broker relays and enforces it.

## Reload and reconnect

No new state. A reload ends the connections (and so their streams and subscriptions) as
today; `bus-reload` adds the drain before the exec (B13).

## Backpressure

This change is the backpressure of the bus: per-stream and per-connection credit, 1024
interactive envelopes and 4 MiB bulk per connection, 256 pending calls per caller, `busy` instead
of growth, events dropped for a lagging subscriber (B7, B8).
