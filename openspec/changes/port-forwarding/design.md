# Design

## Context

At 1163abd: server-initiated streams carry the SSH agent (`crates/tilrd/src/agent.rs`): `StreamOpen{stream_id, kind, pane_id}`, `StreamData` (≤ 64 KiB) both ways, `StreamClose`; at most 64 streams per client; server-to-client data on the bulk lane (read from the program only as the lane has room); client-to-program data queued at most 64 messages, beyond which the stream is ended. The SOCKS proxy (`crates/tilrd/src/socks.rs`) serves CONNECT on `/socks` over WebSocket or raw SOCKS5 on the TCP port, off unless `--socks`.

## Decisions

### D1. -R is new on the server; -L rides on SOCKS
A local forward is "connect from the server's machine to HOST:PORT", which `/socks` already does with its own switch, token check and Origin check; `tilr forward -L` is a client of it. A remote forward needs the server to listen, which nothing does today: that is the new part. Rejected: a client-initiated stream kind for -L (a second way to open outbound connections, bypassing the SOCKS switch the operator chose).

### D2. Messages
`ForwardListen { request_id, port }` → `ForwardListening { request_id, forward_id, port, error }` (`port` the bound one). Per accepted connection: `StreamOpen { stream_id, kind: "tcp-forward", forward_id }` (new field; `pane_id` empty), then the agent stream rules. `ForwardStop { forward_id }` closes the listener; its open streams go on until closed. The client maps `forward_id` to its local target; the server never sees HOST:HPORT of the client's side.

### D3. Where and who
The listener binds `127.0.0.1:<port>` only. Any local user can connect to a loopback port (unlike the agent's 0600 socket); hence the switch and the default in the Блокер. The forward belongs to the client that asked: it ends when that connection ends (disconnect, 4000 superseded, 4001, 4002, reload). At most 16 forwards per client (`too many forwards`); the 64-stream limit counts agent and forward streams together; a connection beyond it is closed at once.

### D4. Feature naming under the protocol-v3 rule
`"forward"` in `HelloAck.features` when the server takes `ForwardListen` (it can be run without). `ForwardListen` to a server without it gets `ForwardListening.error` `forwarding is turned off on this server`. Client side, `"forward"` in `Hello.features` says the client takes `tcp-forward` streams; a client that asks for a forward must list it.

### D5. tilr forward
`tilr forward -L [ADDR:]LPORT:HOST:HPORT` (ADDR defaults to 127.0.0.1) and `-R PORT:HOST:HPORT`, repeatable, connecting as `tilr attach` does (`--connect` / `TILR_CONNECT`, token rules of `tilr-cli`); -L needs a TCP target (the SOCKS path is not on the Unix socket) and says so otherwise. It prints `forwarding 127.0.0.1:<port> on the server to HOST:HPORT` per -R, reconnects with backoff after a loss and asks for its forwards again; Ctrl+C stops it.

## Reload and reconnect

Listeners are not handed over: at the exec every client connection closes and with it its forwards, and open streams end (the programs read end of file), as agent streams do. The successor starts without forwards; `tilr forward` reconnects and asks again (the same port; if another process took it meanwhile it reports the error and keeps the others). No state format change.

## Backpressure

As agent streams: server-to-client data read from the TCP connection only as the client's bulk lane has room (4 MiB per client for all bulk data), so a slow client holds the connecting program back; client-to-program data queued at most 64 messages (about 4 MiB) per stream, beyond which the stream ends with `StreamClose`. Memory per client is bounded by the bulk window plus 4 MiB per open stream, at most 64 streams.
