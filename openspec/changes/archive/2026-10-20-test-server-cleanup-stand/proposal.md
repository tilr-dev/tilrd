# Test-server cleanup stand (TILR-162)

## Why

TILR-162 records Dan's decision to split the deterministic signal/timeout
stand from TILR-153. The latter needs a way to confirm a test-owned tilrd is
running, interrupt its test process, and show whether that exact server
survives. The earlier suite runs did not reproduce or explain the leak;
TILR-153 now tracks that investigation in `test-server-leftovers`.

## What Changes

- Add an ignored Rust integration fixture that starts the real tilrd, waits
  for its `READY=1` notification, and reports its PID to an external stand.
- Add a Python stand with separate SIGTERM and watchdog timeout cases. It
  checks the owned PID (and macOS `pgrep`), then cleans its process group.
- Add synthetic integration checks for leak detection, a clean shutdown,
  and cleanup when readiness or a process check fails.

## Non-goals

- Fixing the test-server lifecycle leak is TILR-153.
- Changing production daemon behavior or the full-suite test runner.

## Compatibility and impact

No breaking change, migration, protocol bump, feature flag, or user-facing
CLI change. The stand is an explicit developer command. It adds no product
state or connection, so nothing needs to survive a reload, update, or client
reconnect. No new outbox traffic or memory pressure is introduced.

