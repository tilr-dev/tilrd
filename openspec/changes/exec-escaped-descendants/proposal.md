# Proposal: Exec limits hold when a descendant escapes the process group

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found (findings #6, #15, #16, #18): a descendant that leaves the group (`setsid`) keeps stdout open and the biased select never polls the deadline, so no `ExecExit` comes, the slot leaks and cancel and the time limit stop working (`exec.rs:568-578,514`); a stand-in `ExecExit` can go ahead of queued output and output can follow `ExecExit` (`exec.rs:370-383,242`); `ExecStart` is silently ignored when exec is off, and `ExecStarted` / `ExecInputAck` can drop on a full interactive lane (`supervisor.rs:2045-2047`, `exec.rs:226-229,623`); `kill(-pid)` after the reap can hit a reused pid (`exec.rs:512`).

## What Changes

- The time limit and `ExecCancel` are honoured even while a pipe stays open; after the program exits, its pipes are drained for at most 1 s and then closed, and `ExecExit` is sent.
- `ExecExit` is always the last message of an exec.
- `ExecStart` with exec off is answered with `ExecStarted` carrying an error.
- `ExecStarted` and `ExecInputAck` are never dropped.
- The group is signalled only while its leader is not yet reaped.

## Capabilities

### Modified Capabilities
- `exec`: see `specs/exec/spec.md`.

## Out of scope

- Tracking processes that leave the group (they are outside the exec by design).
- Changing the 4 MiB stdin window.

## Impact

`crates/tilrd/src/exec.rs`, `supervisor.rs`. Nothing crosses a reload (execs end at a reload); additive behaviour only, no protocol change.
