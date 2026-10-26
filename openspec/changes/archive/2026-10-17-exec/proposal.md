# Proposal: Exec: run a program without a shell over the protocol

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-077 was red: there was no Exec (itests/USE_CASES.md).

## What Changes

- `ExecStart` (argv, cwd, env, stdin, a time limit) runs a program without a shell in a process group of its own; `ExecStarted`, `ExecOutput` (stdout and stderr apart, through the bulk lane), `ExecExit` with the code, the signal and why it was killed.
- Stdin has a 4 MiB window acked by `ExecInputAck`; `ExecSignal`, `ExecCancel`.
- The group is killed on a time limit, cancel, the program's own exit, the client going, a reload and shutdown.
- `--no-exec` and `exec = false` turn it off; feature `exec`.
- `tests/exec.rs` and itests UC-077 (10 cases).

## Capabilities

### Modified Capabilities
- `exec`

## Out of scope

- Descendants that escape the group (open change `exec-escaped-descendants`).

## Impact

Implemented by: [`72af51d`](https://github.com/tilr-dev/tilrd/commit/72af51d) Exec: a client runs a program without a shell … (2026-10-17). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
