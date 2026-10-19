# Proposal: No test servers left running after cargo test

## Why

Test servers from `target/debug` were left running on the Mac after local cargo tests (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`). They get in the way of the next runs and hide flakes.

## What Changes

- Find which test or harness path leaves them (a panicking test, `cargo test` interrupted by a signal, a test that times out) and fix it; each case is created deterministically by the test (the stand of TILR-162), not caught by repeated runs.

## Capabilities

None: tests only, no requirement changes.

## Impact

The harness in `crates/tilrd/tests/common` and `crates/tilrd/tests/local.rs`. No behavior, protocol, reload or update impact. Tracker: TILR-153 (epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
