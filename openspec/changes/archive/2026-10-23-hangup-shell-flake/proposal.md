# Proposal: Fix the a_server_started_from_a_terminal_outlives_the_terminal_hanging_up flake

## Why

`reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up` failed once on macOS under load (2026-10-18), "did not happen: the login shell exits with its terminal" at `tests/common/mod.rs:782`; green alone. The wait was widened to `TIMEOUT` without a cause found (design.md); that hides the cause, so the task was reopened (TILR-117, 2026-10-19).

## What Changes

- Find what holds the login shell after the terminal hangs up, reproduce it deterministically with the hook of TILR-157 (holding SIGHUP delivery or the shell's exit), fix it by its cause; the test waits for the shell's exit as an event, not for a widened deadline.

## Capabilities

None: tests only, no requirement changes.

## Impact

`crates/tilrd/tests/reconnect.rs` and the harness in `crates/tilrd/tests/common`. No behavior, protocol, reload or update impact. Tracker: TILR-117 (epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
