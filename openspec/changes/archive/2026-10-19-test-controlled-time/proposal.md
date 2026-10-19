# Proposal: control server time in integration tests

## Why

TILR-158 and TILR-148 record the need for timing, pane-use and stalled-SOCKS tests with a known elapsed interval. Wall-clock sleeps cannot establish their ordering deterministically.

## What Changes

- Under `failpoints`, a server and its pane processes may read a shared test clock file. Its value supplies the pane's timing measurements and the supervisor's `PaneInfo` timestamps.
- Under `failpoints`, the shared clock can expire a raw SOCKS negotiation deadline early. The ten-second real-time limit remains in force.
- A build without `failpoints` uses the existing clocks and ten-second timeout regardless of these test settings.

## Capabilities

### Added Capabilities

- `test-controlled-time`: test-only clock and SOCKS deadline controls.

## Impact

`crates/tilrd/src/time.rs`, `pane.rs`, `supervisor.rs`, `socks.rs`, integration tests and `USE_CASES.md`. No protocol change or release setting.

## Out of scope

- Production clock behavior, WebSocket SOCKS deadlines and scheduling timers.
- Persisted clock state or client protocol changes.

## Reload and update

The test clock path is an environment setting and the file is read at each observation. It adds no state to hand to a successor on reload and no client state to restore on reconnect. A release update continues to use the existing real-time behavior without `failpoints`.

## Implementation

Commit `7e63724` implements the change.
