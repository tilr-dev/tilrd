# Design

## Context

The bulk lane is paced by pongs to pings that carry a byte position (`outbox.rs`); a ping without a position is also sent for liveness (`supervisor.rs:1048`). WebSocket pongs may answer only the latest ping (RFC 6455 allows a pong to the most recent ping only), so a position-less ping after the marked one can leave the window closed for ever.

## Decisions

- D1: while the window is closed, every ping (liveness ones too) carries the current position; when open, positions every 128 KiB as now.
- D2: the full frame of a lagging client is queued with a notify of the writer task, the same path as ordinary frames.
- D3: `acked = min(pong_position, sent)`.

## Backpressure and memory

Bounds are unchanged: 4 MiB queued per client and 1 MiB in the network unread. D3 removes the only way a client could make the server believe less is in flight.

## Reload and reconnect

No state crosses a reload; a reconnect starts a fresh outbox, as now.
