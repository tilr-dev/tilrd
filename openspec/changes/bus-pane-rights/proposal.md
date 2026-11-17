# Proposal: Pane rights on the bus

Зависит от: bus-core-panes, bus-rights

Стартует после: bus-broker-core (contract: `AuthzRequest::PaneCreate`/`PaneAccess`); мержится после: bus-core-panes, bus-rights

Владеет: `crates/tilrd/src/bus/core/pane_rights.rs`, the `PaneCreate`/`PaneAccess` checks in
the Biscuit authorizer (`bus/acl.rs`, after `bus-rights`), the call sites in
`bus/core/panes.rs` (after `bus-core-panes`), `tests/common/bus_panes.rs` additions in its own
block, its block of `crates/tilrd/tests/bus_terminal.rs`.

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` T7, T8,
R3, R9, O3): `pane_access` defaults to `own`; creating panes needs pane rights (a right to create
panes of profile X is the "sandbox only" token); panes outlive their creator.

## What Changes

- The core's second authorization with pane facts (`pane_profile_requested`, `pane_overrides`,
  `pane`, `pane_creator`, `caller`; T7) for `pane.create` and every pane-scoped method.
- Pane-scoped events (`scope` `pane:<id>`) filtered per subscriber by the same check (B6, R6).
- Panes keep running when their creator stops; its next connection accesses them again (T8).

## Capabilities

### New Capabilities
- `bus-terminal`: creating a pane needs pane rights; pane access follows the creator; a pane
  outlives the participant that made it. ("A pane keeps its creator across a reload" is
  `bus-reload`'s; the reload clause of "Pane access follows the creator" moved there in the
  split.)

## Non-goals

- `close_with_creator` (shared design); the owner's ambient access (unchanged).

## Impact

`bus/core/pane_rights.rs`, `bus/acl.rs`, `bus/core/panes.rs`, tests. Closes the gap of
`bus-core-panes` (every allowed caller saw every pane). Protocol: version 1, no flag. Reload:
nothing new (`created_by` crosses with `bus-reload`). Nothing released from `main` (O5).
