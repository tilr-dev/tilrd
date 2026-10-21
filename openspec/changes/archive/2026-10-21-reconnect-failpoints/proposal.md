# Proposal: Control reconnect ordering in integration tests

## Why

The reconnect tests need explicit control over output produced without a viewer,
connection acceptance, subscription, full frames and the slow-client decision.
The existing failpoint pause controller provides the release handshake.

## What Changes

- Add named pause points along the reconnect path, documented beside their calls.
- Let integration tests pause socket reads until they explicitly resume them.
- Exercise reconnect ordering and client lag using events rather than elapsed time.

## Capabilities

### New Capabilities
- `reconnect-testing`: deterministic reconnect and slow-client test controls.

## Impact

Test-only failpoints in the daemon and helpers in `tests/common`; no protocol,
release behavior, reload state or update compatibility changes. Pause state is
local to each hit and is not handed over through a reload.
