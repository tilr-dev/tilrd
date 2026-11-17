# Proposal: Credits, lanes, limits and deadlines on the bus

Зависит от: bus-broker-core

Стартует после: bus-broker-core; мержится после: bus-broker-core

Владеет: `crates/tilrd/src/bus/{events,streams}.rs` (after `bus-broker-core`),
`crates/tilrd/src/bus/broker.rs` and `crates/tilrd/src/bus/transport.rs` (after
`bus-broker-core`, before `bus-reload`: merge order bus-broker-core → bus-streams-events →
bus-reload; no other change edits these two files),
`tests/common/bus_streams.rs`, its block of `crates/tilrd/tests/bus.rs`, README "The bus"
streams subsection and the `tilrd_bus_events_dropped_total` line in README "Metrics".

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md` B5-B8, O3):
one-way streams with backpressure, lossy events for laggards, bounded queues and no global
lock. The skeleton of `bus-broker-core` routes but does not bound; the core's streams
(`pane.output`, `fs.read`, `exec.run`) need these bounds before they land.

## What Changes

- HTTP/2-style byte credits per stream and per connection, end to end through the broker
  (`Call.window`, `Credit`, `HelloAck.stream_window`, `connection_window`); a provider that
  overruns its credit gets its stream ended with `bad_reply` and `Cancel` (B7).
- Interactive and bulk lanes per `MethodSpec.lane` on the existing `outbox.rs`; bulk bounded at
  4 MiB queued and 1 MiB uncredited per connection (B7, B8).
- `max_in_flight` (256) pending calls per caller and full callee queues answered `busy`;
  deadlines (`min(timeout_ms, call_timeout_ms)`, time left forwarded), `Cancel` on timeout or a
  caller's end (B5); the full set of error answers.
- Events dropped, not queued, for a subscriber whose lane lags; metric
  `tilrd_bus_events_dropped_total{subject}` (B6); no global lock, one timer wheel (B8).
- `StreamOut::send` waits for credit and `is_lagging` also reports spent credit (contract of
  `bus-broker-core`).

## Capabilities

### New Capabilities
- `component-bus`: why a call failed, deadlines and cancellation, lagging subscribers, credit,
  the per-connection bulk bound, lanes, slow participants. (The capability is shared with the
  other bus changes; each adds its own requirements.)

## Non-goals

- Instances, rights, the core's methods; replay or durable events (shared design, Non-goals).

## Impact

`bus/{events,streams,broker,transport}.rs`, `crates/tilrd/tests/bus.rs`, README. Protocol: version 1, no flag;
new metric. Nothing crosses a reload (streams end `unavailable` on a reload, B13, implemented
with `bus-reload`; until then a reload simply ends the connections). Nothing released from
`main` (O5).
