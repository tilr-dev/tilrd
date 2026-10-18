# Proposal: Each server process has its own host-events socket

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #11): the host-events socket path is keyed by `server_id` (`host.rs:114-120,132`), so two `tilrd` of one user (two sockets, one state directory) share the path and unlink each other's socket.

## What Changes

- The host socket path names the serving process (proposed: `<server_id>-<pid>.sock`; the pid survives a reload in place), so two servers of one user never share it.
- `TILR_SOCK` of each pane names its own server's socket; stale sockets of dead pids are swept at start.

## Capabilities

### Modified Capabilities
- `clipboard-host-events`: see `specs/clipboard-host-events/spec.md`.

## Out of scope

- Giving two servers of one user different server ids.

## Impact

`crates/tilrd/src/host.rs`. Has to survive a reload (same pid, same path) and an update from the previous release: a reloaded old pane keeps its old `TILR_SOCK`, so the successor keeps serving the old path until those panes end. No protocol change.
