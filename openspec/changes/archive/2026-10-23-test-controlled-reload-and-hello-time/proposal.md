# Test-controlled reload and Hello deadlines

## Why
Rust integration tests currently wait ten real seconds to exercise a stalled successor check or missing Hello. The shared test clock can make these cases deterministic while preserving the real upper bounds.

## What Changes
- Add test-clock deadlines and local wake-up ports for successor checks and Hello handshakes in failpoints builds.
- Require explicit acknowledgement of test-clock expiration so a test cannot pass through the real-time timeout instead.
- Keep existing test names and production ten-second limits. Black-box itests (UC-039 and reload scenarios) continue to verify the real ten-second behavior.

## Impact
- Affected specification: test-controlled-time (ADDED requirements only).
- Affected code: shared time hooks, reload checking, Hello handshake, and Rust integration tests.
