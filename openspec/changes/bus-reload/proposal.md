# Proposal: The bus across tilrd's reload

Зависит от: bus-rights, bus-core-panes, bus-pane-rights, pane-profiles, sandbox-linux

Стартует после: bus-instances (handover of the runner's instances) and `pane-profiles` task 1.1; мержится после: bus-rights, bus-core-panes, bus-pane-rights, pane-profiles, sandbox-linux

Владеет: `crates/tilrd/src/reload.rs`, `crates/tilrd/src/bus/state.rs`, the state formats and
`tilrd state-formats`, `crates/tilrd/proto/internal.proto` (created: the internal schemas moved
out of `tilr.proto`, which keeps only `tilr.v1`'s client messages until `remove-tilr-v1`), the
drain in `bus/broker.rs` (after `bus-streams-events`), its block of `crates/tilrd/tests/reload.rs`
and `tests/input_delivery.rs`, README "Reloading without closing shells".

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` B13, M3,
O3): instances survive tilrd's reload; panes keep their profile snapshot, creator and sandbox;
moving from a build before the bus to a bus build is a restart, not a reload.

## What Changes

- `bus-state 1` in `server-<pid>.state`: instances, their connection and listener fds, provides,
  counters, tokens' revocation ids and renewal times, subscriptions (B13); the drain before the
  exec (calls answered `unavailable`, `Cancel` to callees, streams ended `unavailable`).
- **BREAKING** (reload state): `supervisor-state 3` with `PaneState.profile` (the snapshot of
  `pane-profiles`), `created_by` and `sandboxed`. A build that writes `supervisor-state 2` is
  not reloaded into this one (the `state-formats` check refuses, logs why and keeps its panes
  and clients); migration: restart tilrd once (M3, owner); open shells end then.
- The internal schemas (pane link, reload state) move from `tilr.proto` to
  `crates/tilrd/proto/internal.proto` (package `tilrd.internal.v1`), with the client-facing
  types they hold taken from `tilr.pane.v1`; the legacy `PaneHandover` fields 16-17 and their
  messages are removed (reserved) *(moved here from 5.2 by the split: this change is the one
  that changes those schemas)*.
- Replaces `bus-instances`' interim reload (instances were stopped and started again).

## Capabilities

### New Capabilities
- `component-bus`: instances survive a tilrd reload.
- `pane-profiles`: a pane keeps its profile across a reload.
- `pane-sandbox`: the sandbox survives a reload.
- `bus-terminal`: a pane keeps its creator across a reload (split out of "Pane access follows
  the creator").

### Modified Capabilities
- `reload`: "state-formats names what this executable reads", "Client connections end and
  clients reconnect" (with the scenario of an outside participant, a Tilr Web page or a script,
  finding its panes after a reload; moved here from `bus-web`).

## Non-goals

- Carrying WebSocket participants' connections across the exec (they reconnect, as clients do
  today); reading `supervisor-state 2` (M3, rejected).

## Impact

`reload.rs`, `bus/state.rs`, `bus/broker.rs`, `supervisor.rs` (state write/read hooks),
`pane.rs`, `internal.proto`, `tilr.proto` (internal messages removed from it; its `tilr.v1`
client messages unchanged), tests, README. This change has to survive a reload between bus
builds; an update from the previous release is a restart (M3; nothing released from `main`, O5).
