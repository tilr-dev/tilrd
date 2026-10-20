# Proposal: Preserve the last integration test run in coverage

## Why

`./itests/dt coverage` builds a local report. When `itests/results/latest/report.json` is absent because integration tests have not run in this folder, it replaces the last-run summary and case statuses with empty values, so the local report loses the statuses it already showed. There is also no way to build the report from a run made in another folder.

## What Changes

- Coverage generation uses the local latest report when present. When it is absent, it carries the last-run summary and known case statuses from the existing output `coverage.json`.
- `--report PATH` selects a report explicitly, including one from another folder.
- The itests README explains the fallback and the option.

## Capabilities

### Modified Capabilities

- `spec-coverage`: adds report selection and preservation of the last recorded run.

## Non-goals

- Running integration tests or copying reports between folders.
- Preserving statuses for cases that are no longer in the current case registry.

## Impact

Capability `spec-coverage`; `itests/tools/coverage.py`, its Python regression test, and `itests/README.md`. This is an offline tool; no server reload, client reconnect, protocol or state migration applies. An update from the previous release can use an existing coverage file immediately. No feature flag or PROTOCOL_VERSION change is needed.
