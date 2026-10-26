# Proposal: The bulk window cannot stall; a lagging client gets its last screen

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found (findings #5, #13, #17). Re-checked against the code at 1163abd (`outbox.rs` and the writer in `supervisor.rs` unchanged in substance):

- #5: once the bulk window is full the writer sends one marking ping (`outbox.rs:309-321`) and then waits for a pong that carries a position (`supervisor.rs:1114`); the liveness pings it also sends are empty (`supervisor.rs:1112`). RFC 6455 lets a peer that has not yet answered several pings answer only the most recent one, and tungstenite keeps only the latest pending pong (review, from memory, untested). A client that does so answers the empty ping, the mark is never acknowledged, no more bulk is written, so no new mark is sent: the window stays closed for ever (file reads, streams and exec output stop; frames go on).
- #13: when a client starts lagging, the supervisor skips the frame, records the lag and asks to hear when the interactive lane empties (`supervisor.rs:2464-2475`, `ToClient::want_drained`, `outbox.rs:219-223`). If the writer emptied the lane between the `lagging()` check and `want_drained()`, `Outbox::written` already returned false (`outbox.rs:254-260`) and no later write comes: no `ClientDrained`, no full frame asked. If the pane then goes quiet, the client keeps a stale screen until the pane prints again.
- #17: `BulkAcks::pong` takes any position with `fetch_max` (`outbox.rs:324-336`); a pong ahead of what was sent opens the window beyond 1 MiB unread (bounded by the 4 MiB lane, so memory is not at risk, but the frame latency the window protects is).

## What Changes

- Every ping the server sends on a main-protocol connection carries the current bulk position (`tilr-bulk:` + big-endian u64), the liveness pings included, so a pong to any ping moves the window (design D1).
- When a client starts lagging and its interactive lane is already empty, the full frame is asked for at once; the 1 s lag tick also asks for one for any lagging client whose lane is empty (design D2).
- A pong's position counts as at most the bulk bytes written to that client (design D3).

## Capabilities

### Modified Capabilities
- `client-liveness`: see `specs/client-liveness/spec.md`.

## Non-goals

- Changing the window (1 MiB), the lane bound (4 MiB), the mark interval (128 KiB) or the ping interval.
- QUIC transport (change `quic-transport`).
- Any protocol change: the ping payload format is already in the `client-liveness` spec; a client echoes ping payloads as WebSocket requires.

## Impact

`crates/tilrd/src/outbox.rs` (`BulkWindow`: a position for every ping, the cap), `crates/tilrd/src/supervisor.rs` (writer's liveness ping; lag start and tick). Not breaking: clients already echo every ping payload. Nothing crosses a reload (the outbox is per connection); an update from the previous release changes nothing on disk.

Implemented by: [`7d18ae3`](https://github.com/tilr-dev/tilrd/commit/7d18ae3) fix: keep the bulk window moving and bring a lagging client its last screen (2026-10-18).
