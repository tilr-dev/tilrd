# Design: The bus crate

Shared design: `openspec/design/unified-bus.md` (B3, B4, B5, B7, B9, T1, T2, T4-T9, P6, R3-R5,
K4). This file holds only what is this change's own.

## Decisions

- **All schemas now.** The payload schemas cover every namespace of T2 and the methods added
  by the phase (`pane.read`, `pane.profiles`, `server.reload`, `server.update`, `clipboard.get`,
  `bus.*`), so that no later change edits a `.proto` (O4). A field found missing later is a small
  PR of its own, merged before the change that needs it.
- **Types copied, not moved.** `tilr.proto` stays untouched until `remove-tilr-v1` (O2); the
  client-facing types are copied into `tilr.<ns>.v1` with the same field numbers, so the bytes a
  pane process writes decode as either (`tilr.v1.Frame` = `tilr.pane.v1.Frame`).
- **Frozen client surface** (what later changes code against; added by the split, from 1.3):
  `Bus::connect(Transport, ConnectOptions) -> Bus`; `Bus::call(method, payload, CallOptions) ->
  Result<Reply, BusError>`; `Bus::stream(method, payload, StreamOptions) -> impl Stream<Item =
  Result<Chunk, BusError>>` (credit returned as chunks are consumed, `drop` cancels);
  `Bus::provide(MethodSpec, handler)` before connect; `Bus::emit(subject, payload)`;
  `Bus::subscribe(pattern) -> impl Stream<Item = Event>`; `Bus::activate()`;
  `Bus::update_credential(Credential)`; `Bus::identity()`, `Bus::hello_ack()`; `BusError {code,
  message}` with B5's codes. Additive changes only after this change.

## Reload and reconnect

Nothing of this change runs in the server. The client reconnects only when asked (the callers
decide, T10).

## Backpressure

The client keeps at most `max_in_flight` calls pending (later ones wait) and returns stream
credit only as the caller consumes chunks (B7); its outbox is bounded as `tilr.v1`'s client is
today (T5).
