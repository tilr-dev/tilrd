# Proposal: An itests case for the manual install from the CDN

## Why

`installation` "Manual install from the CDN" has no test (`openspec/coverage/coverage.md`, regenerated 2026-10-18): the README's manual steps are checked by nothing.

## What Changes

- A new itests case UC-007 doing the README's manual steps against `LocalCdn`, named in the requirement's **Verified by** line (spec delta below).
- A case that finds the code differs from its requirement is kept red and the difference goes to its own change (no behavior change here).

## Capabilities

### Modified Capabilities
- `installation`: "Manual install from the CDN" **Verified by** names the new case; requirement text unchanged.

## Non-goals

- Behavior changes; the release scripts (change `release-process-tests`).

## Impact

Tests only: `itests/scenarios/uc_007.py`, itests/USE_CASES.md, the `installation` spec's **Verified by** line and Notes. No behavior, protocol, reload or update impact. Tracker: TILR-150 (epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
