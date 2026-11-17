# Design: Deno, browser and Python libraries for the bus

Shared design: `openspec/design/unified-bus.md` Part K (K1-K5), §B4-B7 (what the libraries
speak), §B9 (protobuf payloads). This change's part follows.

## Decisions

- **Generated from the frozen schemas.** `scripts/gen-bus-sdk.sh` reads `crates/tilr-bus/proto/`
  as `bus-wire` left it; the SDKs never add a schema (O4).
- **Test runtime per SDK**, plus the Rust one of `bus-wire` for cross-language tests.

## Reload and reconnect

On a reload, WebSocket connections of the SDKs end and the library reconnects only when the
application asks (as `tilr-bus`); instance connections cross the reload with `bus-reload` and
need nothing from the library. A `CredentialUpdate` replaces `bus.credential`.

## Backpressure

Writes wait for the socket (Deno `drain`, Python `await writer.drain()`, browser
`bufferedAmount`); at most `max_in_flight` own calls pending; `out.write` of a stream provider
waits for credit; credit is returned as chunks are consumed (K3).
