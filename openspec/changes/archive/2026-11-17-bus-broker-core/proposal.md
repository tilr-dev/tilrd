# Proposal: The bus broker in tilrd beside tilr.v1

Зависит от: bus-wire

Стартует после: bus-wire; мержится после: bus-wire

Владеет: `crates/tilrd/src/bus/mod.rs`, `crates/tilrd/src/bus/core/mod.rs` and every `mod`
declaration under `crates/tilrd/src/bus/` (all submodules created here as stubs, design
"Module tree"); `bus/{broker,transport,identity,authz}.rs`; the first versions of
`bus/{events,streams}.rs`; `crates/tilrd/tests/common/bus.rs`, the stub files
`tests/common/bus_<lane>.rs` and their `pub mod` lines in `tests/common/mod.rs` (nothing else
in that file); `crates/tilrd/tests/bus.rs` (created); README "The bus" (created with empty
subsections); the `tilr-bus` dependency of `tilrd`.

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md`, Owner's
decisions, O1-O4). The broker and its extension points come before every lane: the frozen
contract in this change's design is what the parallel changes code against, so that they do not
edit each other's files.

## What Changes

- The broker skeleton (B1, B2, B4, B5, B6, B7): routing of `Call`/`Reply` by target or method
  (`ROUND_ROBIN`, `LAST_ACTIVE` with `Activate`), `from` set by the broker, replies accepted only
  from the callee's connection, reserved namespaces, per-pair order; plain streams (`Chunk*`,
  `End`, error `Reply`, caller's `Cancel`) and events (NATS patterns, delivery to the subscribers
  connected now) without credits, bounded queues or deadlines (those are `bus-streams-events`).
- The WebSocket transport of the bus on the TCP port and the Unix socket under the
  subprotocol `tilr.bus.v1` (B3, B12): Hello/HelloAck, `ConnectionError`, owner by peer uid or
  access token, liveness and superseding kept, `bus.ping`. A handshake without the subprotocol
  is served `tilr.v1` exactly as before (O2).
- The extension points of the design (types, traits, the native participant `tilrd`, the
  supervisor handle, a stub authorizer, the test harness), frozen after this change.

## Capabilities

### New Capabilities
- `component-bus`: Hello and version, unknown and bad envelopes, the broker's `from`, routing,
  per-pair order, streams as calls answered in chunks, events to the subscribers connected now.
  The other requirements of `component-bus` are added by `bus-streams-events`,
  `bus-instances`, `bus-core-panes`, `bus-reload`, `bus-tilr-client` and `remove-tilr-v1`.

## Non-goals

- Credits, lanes for bulk, `busy`, deadlines (`bus-streams-events`); instances
  (`bus-instances`); Biscuit (`bus-rights`); the core's methods (`bus-core-*`).
- Any change to `tilr.v1`'s behavior; refusing `tilr.v1` clients (`remove-tilr-v1`).

## Impact

`crates/tilrd/src/bus/` (new), a hook in the existing WebSocket accept path (subprotocol
selection), `crates/tilrd/tests/`, README. Protocol: `tilr.bus.v1` version 1 (B14) is served in
addition to `tilr.v1`; no **BREAKING** change for `tilr.v1` clients; no feature flag. Reload: a
bus connection ends on a reload like a `tilr.v1` client's and reconnects; nothing new crosses a
reload. Update from the previous release: nothing is released from `main` (O5).
