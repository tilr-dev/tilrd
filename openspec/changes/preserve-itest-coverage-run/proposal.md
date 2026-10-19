# Proposal: Preserve the last integration test run in coverage

## Why

TILR-163 records that `./itests/dt coverage` currently replaces case statuses and the last-run summary with empty values when `itests/results/latest/report.json` is absent in a fresh worktree. This caused unrelated coverage changes during TILR-155 review on 2026-10-19.

## What Changes

- Coverage generation uses the local latest report when present. When it is absent, it carries the last-run summary and known case statuses from the existing output `coverage.json`.
- `--report PATH` selects a report explicitly, including one outside the worktree.
- The itests README explains the fallback and the option.

## Capabilities

### New Capabilities

- `itest-coverage`: report selection and preservation of the last recorded run.

## Non-goals

- Running integration tests or copying reports between worktrees.
- Preserving statuses for cases that are no longer in the current case registry.

## Impact

`itests/tools/coverage.py`, its Python regression test, `itests/README.md`, and regenerated `openspec/coverage/`. This is an offline tool; no server reload, client reconnect, protocol or state migration applies. An update from the previous release can use an existing coverage file immediately. No feature flag or PROTOCOL_VERSION change is needed.
