# Proposal: Preserve cleanup after a failed test log wait

## Why

TILR-170: the command-line rejection reload test timed out in `state-formats`.
Its missing-log assertion then poisoned the log mutex; the log reader and
`Server::drop` panicked, turning the original failure into SIGABRT.

## What Changes

- Keep the unproven state-formats timeout in known-flakes, as approved by Dan
  on 2026-10-20 (TILR-170 comment #3750); this change fixes the confirmed
  secondary cleanup failure.
- Release the log mutex before reporting a failed wait, and preserve cleanup and
  diagnostics even when another log user has poisoned the mutex.
- Exercise failure unwinding in subprocesses so an abort is observable without
  terminating the test suite.

## Capabilities

None for the harness fix: tests only, no product requirement changes. Any probe
behavior change still needs a requirement/scenario once its cause is known.

## Impact

The tilrd integration harness and its regression tests. The separate active
`reload-exec-checked-file` change belongs to another task and is not modified.
