# Proposal: A shell that ignores SIGHUP does not outlive the server

## Why

itests UC-086 is listed as a bug ("Сейчас баг — FAIL") without a scenario: when the server stops, a program in a pane that ignores SIGHUP is left running as an orphan. The requirement "No shell outlives the server" is in the `daemon-lifecycle` spec and has no test (itests/USE_CASES.md group H; `daemon-lifecycle` and `panes` spec Notes).

## What Changes

- On stop, a pane process sends SIGHUP to its shell's process group, waits a grace period (proposed 2 s) and then SIGKILLs the group, before it exits.
- A scenario for UC-086.

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`

No spec delta: the requirements are already in `openspec/specs/` (currently unmet); `skip_specs: true`.

## Out of scope

- Processes that left the pane's session (`setsid`, daemons started from the shell).

## Impact

`crates/tilrd/src/pane.rs`. Applies to stop and `kill-server`, not to a reload (shells must survive a reload). The requirement is already in the spec, so this change has no delta. No protocol change.
