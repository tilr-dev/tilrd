# Proposal: Tests for the untested tilr client requirements

## Why

`openspec/coverage/coverage.md` (`./itests/dt coverage`, regenerated 2026-10-18) listed `tilr-cli` 2 and `update` 2 requirements without a test: `crates/tilr/tests/client.rs` ran neither `tilr kill` nor `tilr update`, and no test compared the client ids of a reconnect.

## What Changes

- `client::reconnects_say_the_same_client_id`, `client::kill_closes_the_named_pane_and_the_current_one` and `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload` (against a local CDN directory, as `crates/tilrd/tests/update.rs` uses), each named in its requirement's **Verified by** line.

## Capabilities

### Modified Capabilities
- `tilr-cli`, `update`: **Verified by** lines name the new tests; requirement text unchanged.

## Impact

Tests only: `crates/tilr/tests/client.rs`, USE_CASES.md, the specs' **Verified by** lines and Notes. Implemented in 2945a95 (TILR-149, epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change) and archived at once: the tests were already on dev.
