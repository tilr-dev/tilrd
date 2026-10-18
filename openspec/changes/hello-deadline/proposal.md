# Proposal: A connection must say Hello within 10 s; un-greeted connections are capped

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #12): the 10 s handshake timeout covers only the WebSocket handshake; a TCP client that completes it and never sends `Hello` is held until the client timeout, and there is no cap on un-greeted connections (`supervisor.rs:981-1019`). itests `uc_039.py::a_websocket_that_never_says_hello_is_dropped_by_the_client_timeout` passes today only on the 30 s client timeout.

## What Changes

- A connection that has not sent `Hello` 10 s after its WebSocket handshake is closed.
- At most a fixed number of connections may be un-greeted at once (proposed 128); further ones are closed at accept and logged once per burst.

## Capabilities

### Modified Capabilities
- `access-control`: see `specs/access-control/spec.md`.

## Out of scope

- A limit on greeted connections.
- Rate limiting token guesses (UC-084).

## Impact

`crates/tilrd/src/supervisor.rs`. Clients that send `Hello` at once (all known ones) are unaffected. Nothing crosses a reload; no protocol change.
