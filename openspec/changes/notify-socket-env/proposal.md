# Proposal: NOTIFY_SOCKET does not leak into panes and execs

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #21): `NOTIFY_SOCKET`, set by systemd for a `Type=notify` unit, is passed on to pane shells and execs (`pty.rs:197-217`), so a program in a pane can send readiness or status messages for the `tilr.service` unit.

## What Changes

- The server removes `NOTIFY_SOCKET` (and `LISTEN_FDS`, `LISTEN_PID`, `WATCHDOG_*`) from the environment of panes and execs after it has notified readiness.

## Capabilities

### Modified Capabilities
- `panes`: see `specs/panes/spec.md`.
- `exec`: see `specs/exec/spec.md`.

## Out of scope

- Nothing beyond What Changes.

## Impact

`crates/tilrd/src/{pty,exec}.rs`. Must hold after a reload: the successor's panes also lack the variables (the supervisor's own environment keeps `NOTIFY_SOCKET` for `READY=1` after a reload). No protocol change.
