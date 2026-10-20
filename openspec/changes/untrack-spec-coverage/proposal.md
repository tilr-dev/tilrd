# Proposal: keep spec coverage reports local

## Why

TILR-172 records Dan's 2026-10-20 decision to stop tracking generated coverage reports: results from individual itests runs change the reports and cause conflicts between test changes (TILR-150).

## What Changes

- Remove `openspec/coverage/coverage.json` and `coverage.md` from git and ignore the directory.
- Keep `./itests/dt coverage` generating the reports locally on demand.
- Document local generation in the runner help, README, OpenSpec config and the active changes named by TILR-172.

## Capabilities

### New Capabilities
- `spec-coverage`: generated coverage reports are local, ignored artifacts.

## Non-goals

- Changing coverage calculation, report formats, registry validation or the last-run fallback (TILR-163).
- Changing server/client behavior, protocol, reload or update state.

## Impact

Repository hygiene and developer documentation only. No breaking runtime change, protocol version or feature flag. Reloads and updates carry no new state.
