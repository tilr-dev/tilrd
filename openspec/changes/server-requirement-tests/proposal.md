# Proposal: Tests for the untested exec, metrics, panes, reload, socks-proxy and ssh-agent-forwarding requirements

## Why

`openspec/coverage/coverage.md` (`./itests/dt coverage`, regenerated 2026-10-18) lists server requirements without a test: `exec` 1, `metrics` 1, `panes` 2, `reload` 1, `socks-proxy` 2, `ssh-agent-forwarding` 1. Checked at 1163abd: none of them has gained a test.

## What Changes

- A cargo test in `crates/tilrd/tests/` per requirement (8), each named in its requirement's **Verified by** line (spec deltas below).
- The time-dependent tests (`metrics` timing fields, `used_at_ms`, the 10 s SOCKS negotiation limit) take their time from the test-controlled clock of TILR-158 (change `test-controlled-time`), not from `sleep`; TILR-158 adds `panes::used_at_ms_follows_subscribe_and_input` and `socks::a_session_that_does_not_say_where_it_goes_is_closed` with it.
- A test that finds the code differs from its requirement is kept red and the difference goes to its own change (no behavior change here).

## Capabilities

### Modified Capabilities
- `exec`, `metrics`, `panes`, `reload`, `socks-proxy`, `ssh-agent-forwarding`: **Verified by** lines name the new tests (see `specs/`); requirement text unchanged.

## Non-goals

- Behavior changes.

## Impact

Tests only: `crates/tilrd/tests/{exec,metrics,panes,reload,socks,agent}.rs`, USE_CASES.md, the specs' **Verified by** lines and Notes. No behavior, protocol, reload or update impact. The deltas are MODIFIED copies of the current requirements with a new **Verified by** line: if another change modifies one of them first, the copy is refreshed before this change is archived (task 2.2). Tracker: TILR-148 (epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
