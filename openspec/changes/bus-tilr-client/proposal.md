# Proposal: Move tilr and tilrd ps to the bus, with tilr bus and profiles

Зависит от: bus-core-services, bus-rights, pane-profiles

Стартует после: bus-core-panes (attach needs `pane.*`); мержится после: bus-core-services, bus-rights, pane-profiles

Владеет: `crates/tilr/**`, `crates/tilr/tests/**`, `crates/tilrd/src/ps.rs` and the `ps`
subcommand path in `crates/tilrd/src/main.rs`, README "tilr".

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` T10, O2,
O3): `tilr` is an outside participant like any other (owner on the Unix socket, or with a
token); scripts reach the bus through `tilr bus call`.

## What Changes

- `tilr` (every subcommand, attach with input replay and reconnect) and `tilrd ps` on the Rust
  client of `tilr-bus` (T10).
- `tilr bus call|sub|ls|start|stop|reload|reload-config` and `tilr bus token|revoke|rotate-key`
  (the methods are `bus-instances`' and `bus-rights`').
- `tilr new --profile`, `tilr profiles` (Part P).

## Capabilities

### New Capabilities
- `component-bus`: `tilr bus`, `tilr bus sub`.
- `bus-acl`: the owner mints tokens (`tilr bus token`).
- `pane-profiles`: `tilr profiles` lists the server's profiles.

### Modified Capabilities
- `tilr-cli`: "Command set" (`profiles`, `bus …`, the bus as its protocol), "new"
  (`--profile`).

## Non-goals

- The test client and harness (`bus-test-clients`); the old `tilr` speaking `tilr.v1` after this
  change (no fallback: the server of the same build speaks both until `remove-tilr-v1`, O2).

## Impact

`crates/tilr`, `crates/tilrd/src/ps.rs`, README. The `tilr` of this build speaks only the bus;
a `tilrd` of an older build (`tilr.v1` only) is not supported by it (nothing released from
`main`, O5; the archive ships `tilr` and `tilrd` together). Reload: `tilr attach` reconnects
and replays input as today (T5).
