# Proposal: A server started from a terminal outlives the terminal

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

itests UC-013 was red: `tilr-server &` from an SSH login died with the session, because bash SIGHUPs the job's process group and the kernel SIGHUPs on the pty hangup; tracing then panicked writing to the hung-up terminal (commit message; Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`).

## What Changes

- The supervisor catches SIGHUP and ignores it, logged once (caught, not `SIG_IGN`, so pane shells still get the default); it does not `setsid` itself (EPERM as a group leader, and a fork would change its pid and break reload in place).
- Every pane process runs in a session of its own, so the shell's SIGHUP to its jobs does not reach it.
- The logger no longer panics on EIO (`log_internal_errors(false)`).
- Test `reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up`; README "Signals".

## Capabilities

### Modified Capabilities
- `daemon-lifecycle`

## Out of scope

- A SIGHUP in the first milliseconds after an exec, before the handler is installed (open change `reload-cli-compatibility`).
- cloudflared in the supervisor's process group (open change `cloudflared-outlives-exec`).

## Impact

Implemented by: [`d52c13a`](https://github.com/tilr-dev/tilrd/commit/d52c13a) A server started from a terminal outlives it … (2026-10-16). Commits on branch `itests` (not yet on `main`; their subjects are to be rewritten before the merge, so the hashes may change).
