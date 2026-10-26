# Proposal: State format versions checked before a reload; tests per use case

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

A successor that cannot read the state it is handed loses every shell; asking the executable which formats it reads is safer than only checking that it runs (commit message).

## What Changes

- Supervisor state and pane resume state carry a format version; state without one is read as format 1.
- Before the exec the supervisor runs the hidden `state-formats` subcommand of the executable on disk (replacing the `--version` check) and does not reload into one that reads other formats.
- `use_cases.rs` split into one file per area with a shared harness in `tests/common`; USE_CASES.md lists every scenario with its test and what is not covered.

## Capabilities

### Modified Capabilities
- `reload`

## Out of scope

- Checking the successor's command line and configuration (open change `reload-cli-compatibility`).

## Impact

Implemented by: [`7e41c18`](https://github.com/tilr-dev/tilrd/commit/7e41c18) Protocol version check, pings, state format versions; tests per use case (2026-10-04).
