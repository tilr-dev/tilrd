# Proposal: Fix the output_printed_while_nobody_watched_is_there_on_reconnect flake

## Why

`reconnect::output_printed_while_nobody_watched_is_there_on_reconnect` (Linux container, under full-suite load) timed out waiting for `printed later` on the screen after the reconnect (`tests/common/mod.rs:324`); green alone. A flake is closed only with its cause established.

## What Changes

- Find why the output is missing after the reconnect, reproduce it deterministically with the reconnect failpoints and the test client's read pause, drop the test's `sleep 1` as synchronization, fix it by its cause.

## Capabilities

None: tests only unless the cause is in the server (then a deterministic test fails before the fix and passes after).

## Impact

`crates/tilrd/tests/reconnect.rs` and the harness in `crates/tilrd/tests/common`. Split from the former shared change `test-coverage-gaps`.
