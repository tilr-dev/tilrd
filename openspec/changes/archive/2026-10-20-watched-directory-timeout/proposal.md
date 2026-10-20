# Preserve watched file changes while awaiting request answers

## Why

TILR-166: `files::a_watched_directory_reports_what_changes_in_it` times out on macOS and Linux. A suspected cause is the test discarding an asynchronous `FsChanged` while awaiting an `FsAck`; this must be demonstrated with a controlled message order before choosing the fix.

## What Changes

- Establish the missing event and its order using event-driven test hooks and timestamped evidence.
- Add a deterministic regression and fix the established cause without sleeps, retries or longer deadlines.
- Verify the complete files target on macOS and Linux.

## Impact

Files integration tests, their harness, and test-only server hooks as needed. No protocol change is intended.
