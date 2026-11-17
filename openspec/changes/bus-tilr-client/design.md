# Design: Move tilr and tilrd ps to the bus

Shared design: `openspec/design/unified-bus.md` §T10 (clients in this repo), §T5 (input,
replay), §B2 (`tilr` as owner or with a token), Part P (profiles). This change's part follows.

## Decisions

- **Same behavior, new transport.** Every subcommand keeps its output and exit status; tests
  in `crates/tilr/tests/` keep their names. `tilr attach` opens `pane.output` with a 4 MiB
  window, sends input as `pane.input` calls without waiting for replies, calls `pane.applied`
  after a reconnect and replays what was not acknowledged.
- **`tilr bus` prints JSON** for the core's methods (`application/json`, B9) and raw bytes
  otherwise.

## Reload and reconnect

`tilr attach` reconnects after a reload as today, with the same `client_id`, and reopens its
stream (full frame first).

## Backpressure

The client's outbox stays ≤ 4 MiB / 4096 inputs (T5); stream credit is returned per decoded
chunk.
