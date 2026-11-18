# Proposal: The core's pane methods on the bus

Зависит от: bus-streams-events

Стартует после: bus-broker-core (contract: `StreamOut`, `BusObserver`, handle); мержится после: bus-streams-events

Владеет: `crates/tilrd/src/bus/core/{panes,json}.rs`, `tests/common/bus_panes.rs`,
`crates/tilrd/tests/bus_terminal.rs` (created; `bus-core-services` and `bus-pane-rights` append
their blocks after it merges), the pane text rendering in `crates/tilr-screen`, the supervisor
hooks named in the design.

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md` T1-T5,
T9, O3): tilrd's core is a native participant and the terminal moves onto the bus. This change
puts panes on it, beside `tilr.v1`, on the supervisor's existing paths.

## What Changes

- The participant `tilrd` provides `pane.create|list|close|resize|input|applied|
  input_finalize`, the stream `pane.output` (frames forwarded unchanged, latest-wins on lag or
  spent credit, `End` on exit, T4), `pane.read` (T9), and emits `pane.created|changed|title|bell|
  command_finished|exited` with scope `pane:<id>`; `seq_base` from `connection_id` (T5).
- `PaneInfo.created_by` is stamped with the caller's identity *(moved here from 4.2 by the
  split: `bus-reload` carries it and `bus-pane-rights` authorizes by it)*.
- Core methods answer `application/json` in the proto3 JSON mapping (B9).
- `tilr.v1` clients keep working unchanged: the same frames reach `tilr.v1` subscribers and bus
  consumers (O2).

## Capabilities

### New Capabilities
- `bus-terminal`: panes created, listed and closed by calls; pane events; `pane.output`;
  lagging watchers; input acknowledgement; numbered input across reconnects; `pane.read`.
- `component-bus`: payloads the broker does not read (and JSON for the core's methods).

## Non-goals

- Pane rights (`bus-pane-rights`): until it lands, every caller the broker lets call `pane.*`
  sees every pane (only the owner and instances whose `rights` name `pane.*` can).
- Profiles (`pane-profiles`), files/exec/clipboard/server (`bus-core-services`), moving clients.

## Impact

`bus/core/{panes,json}.rs`, hooks in `supervisor.rs` and `pane.rs`, `crates/tilr-screen`
(text rendering for `pane.read`), tests. Protocol: `tilr.bus.v1` version 1, no flag; `tilr.v1`
unchanged. Reload: panes cross as today; `created_by` does not cross until `bus-reload`
(after a reload it is `owner`) *(gap named by the split)*. Nothing released from `main` (O5).
