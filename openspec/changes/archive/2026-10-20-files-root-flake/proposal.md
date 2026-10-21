# Proposal: Fix the files::nothing_outside_the_shared_directories_is_served flake

## Why

Known flake (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`): under load `files::nothing_outside_the_shared_directories_is_served` fails; green when run alone. A flake is closed only with its cause established.

## What Changes

- Find the cause, reproduce it deterministically (the test sets up the order of events itself, no load, no `sleep`), fix it; quarantine only with the cause recorded in USE_CASES.md. A product bug found on the way goes to its own task.

## Capabilities

None: tests only, no requirement changes.

## Impact

`crates/tilrd/tests/files.rs` and the harness in `crates/tilrd/tests/common`. No behavior, protocol, reload or update impact. Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
