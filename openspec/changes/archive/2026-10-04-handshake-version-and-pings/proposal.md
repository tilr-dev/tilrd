# Proposal: Hello first with a protocol version; pings and a client timeout

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Rationale not recorded beyond the commit message: a client of another protocol version, or one that sends something else first, must be told so, and silent disconnects must be noticed in both directions, since a page in a browser cannot see WebSocket pings.

## What Changes

- `Hello` must come first and carry the server's protocol version; another version, or another message first, gets an `Error` and a disconnect.
- The server pings its clients over WebSocket and drops one silent for `--client-timeout` (30 s, 0 turns it off).
- `Ping`/`Pong` in the protocol (feature `ping`) for clients that cannot see WebSocket pings.

## Capabilities

### Modified Capabilities
- `protocol`
- `client-liveness`

## Out of scope

- Judging a client by the pongs to the server's own pings (see `2026-10-17-client-liveness-and-supersede`).

## Impact

Implemented by: [`7e41c18`](https://github.com/tilr-dev/tilrd/commit/7e41c18) Protocol version check, pings, state format versions; tests per use case (2026-10-04). The same commit also holds `2026-10-04-state-format-versions`.
