## Why

On 2026-10-31, the owner observed orphaned fake-server clients left running
after successful integration test runs. The terminal harness neither reaped
clients on drop nor prevented them from inheriting PTY master descriptors.

## What Changes

- Own each terminal client until it has exited and been reaped, including panic unwind.
- Open PTY descriptors close-on-exec, preserving only the child's standard streams.
- Explicitly detach and wait in both reconnect tests that omitted cleanup.
- Add regression coverage for cleanup and descriptor inheritance.

## Capabilities

### New Capabilities
- `test-client-lifetime`: integration-test ownership of terminal clients.

## Non-goals

- Product reconnect behavior, protocol changes, and server ownership changes.
- Recovery from arbitrary process termination beyond closing the test's PTYs.

## Impact

Test harness and test documentation only. No breaking changes, protocol version
change, feature flags, persistent state, reload or update migration.

## Implementation

Implemented through terminal ownership, atomic PTY close-on-exec, explicit
reconnect-test cleanup, and regression coverage.
