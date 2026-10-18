# Proposal: A connection must say Hello within 10 s of its handshake

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #12): the 10 s handshake timeout covers only the WebSocket handshake; a client that completes it and never sends `Hello` is held until the client timeout, and there is no cap on un-greeted connections. Re-checked at 1163abd: `HANDSHAKE_TIMEOUT` (`supervisor.rs:71`) bounds the first byte (`supervisor.rs:947-953`) and the WebSocket handshake (`supervisor.rs:1045-1050`) only; after it, the reader loop (`supervisor.rs:1170-1215`) applies the client timeout alone, which is off with `--client-timeout 0`, so an un-greeted connection is then held for ever. itests `uc_039.py::a_websocket_that_never_says_hello_is_dropped_by_the_client_timeout` passes only because it sets `--client-timeout 4`.

## Decision (2026-10-18, technical)

- A connection that has not said `Hello` 10 s after its WebSocket handshake is closed with close code 4002 `timeout`, whatever the client timeout.
- No cap on the number of un-greeted connections (the first draft proposed 128). A cap lets anyone who can reach the port lock legitimate clients out by holding 128 silent connections, each now held at most 20 s (10 s to the handshake, 10 s to `Hello`); UC-038 shows a thousand silent connections leave the server working. The access-control spec keeps "The server MUST NOT otherwise limit the number of connections".

## What Changes

- The 10 s limit extends from the handshake to `Hello`: a connection on the TCP port or the Unix socket that has not sent a `Hello` (of any version) within 10 s of completing its WebSocket handshake is closed with WebSocket close code 4002 and reason `timeout`; the log says `client sent no Hello in 10s`.
- README "Connections" (or "Disconnects") says so.

## Capabilities

### Modified Capabilities
- `access-control`: see `specs/access-control/spec.md`.

## Non-goals

- A limit on greeted or un-greeted connections.
- Rate limiting token guesses (UC-084).
- A new close code (4002 `timeout` is reused; the close codes are part of the contract).

## Impact

`crates/tilrd/src/supervisor.rs` (`serve_websocket`'s reader loop: a Hello deadline told by the supervisor when the client is greeted, or a supervisor tick that closes un-greeted clients older than 10 s). Implemented as the supervisor's once-a-second tick, so the close comes 10-11 s after the handshake; the drop is not counted in `tilr_client_drops_total` (its reasons are unchanged). All known clients send `Hello` at once, so none is affected. Nothing crosses a reload (connections are reopened after one); no protocol change; an update from the previous release changes nothing on disk.

Implemented by: [`f32643d`](https://github.com/tilr-dev/tilrd/commit/f32643d) fix: close a connection that says no Hello within 10 s (2026-10-18).
