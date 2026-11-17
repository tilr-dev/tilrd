# Proposal: Remove tilr.v1 and the tilr-proto crate

Зависит от: bus-wire, bus-broker-core, bus-streams-events, bus-instances, bus-core-panes,
bus-core-services, bus-rights, bus-pane-rights, pane-profiles, sandbox-linux, sandbox-macos,
bus-sdk, bus-test-clients, bus-tilr-client, bus-reload, bus-web (every other Phase 7 change)

Стартует после: bus-test-clients and bus-tilr-client (task 1.4, the spec text, may start after bus-broker-core); мержится после: every other Phase 7 change

Владеет: `crates/tilr-proto/**` (deleted), the `tilr.v1` serving paths in `crates/tilrd/src/`
(deleted), the workspace member removal, the refusal packet, the CI step
`! test -e crates/tilr-proto`, `crates/tilr-screen` and pane process imports, README "Protocol",
the `tilr-proto` and protocol lines of AGENTS.md, the `tilr.proto` lines of
`openspec/config.yaml`, the 11.2 deltas of the main specs (except the requirements other
changes own, design O3).

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` O1, O2,
B3, T1): the bus is the only protocol; `tilr.proto` is removed whole; no adapter. Every client
and test of this repository speaks the bus by now (`bus-test-clients`, `bus-tilr-client`), so
`tilr.v1` can go.

## What Changes

- **BREAKING** (client protocol): `tilr.proto` (package `tilr.v1`), the `tilr-proto` crate and
  `tilr.v1`'s `PROTOCOL_VERSION` are removed. A WebSocket handshake without the subprotocol
  `tilr.bus.v1` gets one `tilr.v1` `Error` saying the server speaks `tilr.bus.v1`, then close 1002
  (B3). Migration: clients move to the bus (this repository's already have; Tilr.app on its own
  release line, M2). The mapping of every message is shared design T2.
- `crates/tilr-screen` and the pane process take their types from `tilr.pane.v1` (T1).
- README "Protocol" rewritten for `tilr.bus.v1` (transports, envelopes, namespaces and T2 in
  short, version rule, features, close codes 4000-4003, the old-client refusal); AGENTS.md and
  `openspec/config.yaml` point at `crates/tilr-bus/proto/`.
- The main specs that name `tilr.v1` messages are rewritten with the bus names of T2, behavior
  unchanged (11.2).
- The release-notes text for the new minor line: moving from a build before the bus is a
  restart of tilrd that ends open shells (M3).

## Capabilities

### New Capabilities
- `component-bus`: a client of the old protocol is told and closed.
- `bus-terminal`: the terminal is served on the bus only.

### Modified Capabilities
- `protocol`: the transport and handshake are the bus's; requirements tied to `ClientMessage` /
  `ServerMessage` replaced, client feature flags and the `tilr.v1` input requirements removed.
- With task 1.4 (11.2), `MODIFIED` deltas for `access-control`, `client-liveness`,
  `clipboard-host-events`, `exec`, `files`, `metrics`, `panes`, `reconnect-testing`, `reload`,
  `screen-frames`, `ssh-agent-forwarding`, `tilr-cli`, `update` (names only; not the
  requirements owned by `pane-profiles`, `bus-reload` and `bus-tilr-client`, design O3).

## Non-goals

- An adapter or serving both protocols (shared design, Non-goals); moving Tilr.app (M2);
  cutting the new minor (the owner's word, O5).

## Impact

`crates/tilr-proto` removed; `crates/tilrd/src/` (serving, link), `crates/tilr-screen`,
`Cargo.toml`, `Cargo.lock`, `.github/workflows/ci.yml`, README, AGENTS.md,
`openspec/config.yaml`, `openspec/specs/**` (by archive). Protocol: `tilr.bus.v1` stays version
1 (B14); feature flags unchanged. Reload: nothing new crosses (`bus-reload` did that); a reload
from a build before the bus is refused (M3). Update from the previous release: a restart, on
the new minor line cut on the owner's word (M2, O5).
