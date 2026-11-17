# Proposal: Move the test client, the harness, the tests and itests to the bus

Зависит от: bus-core-panes, bus-core-services

Стартует после: bus-core-panes (the contract alone runs no test); мержится после: bus-core-panes, bus-core-services

Владеет: `crates/tilr-driver/**`, `crates/tilrd/tests/common/mod.rs` (whole), every test file
of `crates/tilrd/tests/` that speaks `tilr.v1` today (moved, names kept), `itests/` (scenarios,
spec tags), itests/USE_CASES.md (its own section).

## Why

Owner's decisions of 2026-11-17 (session TILR-892; `openspec/design/unified-bus.md` T10, O2,
O3): clients and tests move to the bus one change at a time; test names stay because they state
use cases. After this change the requirements of `bus-core-panes` and `bus-core-services` are
verified by the test names they list, on the bus.

## What Changes

- `tilr-driver` on the Rust client of `tilr-bus`; the itests scenarios drive it as before; spec
  tags updated where a requirement was renamed or moved (`@case(spec=[...])`).
- `crates/tilrd/tests/common/mod.rs`: `common::Client` over `tilr-bus`, with the same helper
  names, so the tests change their messages, not their structure.
- Every test in `crates/tilrd/tests/` moved to the bus with its name kept, except the four
  protocol tests that `remove-tilr-v1` rewrites (`panes::a_malformed_packet_gets_an_error_and_the_connection_stays`,
  `panes::a_message_type_the_server_does_not_know_is_ignored`,
  `panes::a_client_that_skips_hello_is_turned_away`,
  `panes::a_client_built_for_another_protocol_version_is_turned_away`), which keep a raw
  `tilr.v1` path (`tests/common/v1.rs`) until then.
- `crates/tilr/tests/` are `bus-tilr-client`'s.

## Capabilities

None (`skip_specs: true`): no behavior changes; the requirements are those of the changes that
serve the bus.

## Non-goals

- The `tilr` client (`bus-tilr-client`); removing `tilr.v1` (`remove-tilr-v1`).

## Impact

`crates/tilr-driver`, `crates/tilrd/tests/**`, `itests/**`. No protocol, reload or update
change. Nothing released from `main` (O5).
