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
- **Frozen client surface** (what later changes code against; added by the split, from 1.3;
  as built in `crates/tilr-bus/src/client.rs`): `Bus::connect(Transport, ConnectOptions) ->
  Result<Bus, BusError>`, `Transport::{Fd, Socket{path, token}, WebSocket{url, access_token,
  credential, origin}, UnixWebSocket{path, credential, origin}}` and `Transport::from_env()`;
  methods are provided before connect with `ConnectOptions::provide(MethodSpec, Handler)`
  (`Handler::unary`, `Handler::typed`, `Handler::stream` with a `StreamWriter` that waits for
  credit), because `Hello.provides` is the only registration (B4); `Bus::call(method, payload,
  CallOptions) -> Result<Reply, BusError>` and `Bus::call_typed`; `Bus::stream(method, payload,
  StreamOptions) -> Result<ChunkStream, BusError>`, `ChunkStream: Stream<Item = Result<Chunk,
  BusError>>` (credit returned as chunks are consumed, `drop` cancels); `Bus::emit(subject,
  payload)`; `Bus::subscribe(pattern) -> Result<Subscription, BusError>`, `Subscription:
  Stream<Item = Event>`; `Bus::activate()`; `Bus::update_credential(Credential)`,
  `Bus::credential()`; `Bus::identity()`, `Bus::hello_ack()`, `Bus::closed()`; `BusError {code,
  message}` with B5's codes. Additive changes only after this change.
  - `Transport::from_env()` takes `TILR_BUS_FD` only if it is an open socket
    (`fcntl(F_GETFD)`, `fstat` `S_IFSOCK`), and once per process: a second take is
    `bad_request`. The lookup it is built on is crate-private, so safe code cannot make an
    `OwnedFd` from an arbitrary number. `Transport`'s `Debug` shows no token.
  - `ConnectOptions::handshake_timeout` (default 10 s) bounds the whole handshake: connect,
    TLS, WebSocket upgrade, `Hello` and `HelloAck`; past it, `timeout`.
  - The reader starts once the connection's handle exists, so a `Call` read together with
    `HelloAck` is answered; a `Call` that finds the connection closing is answered
    `unavailable`. A handler's panic, before or after it returns its future, is
    `handler_error`.
  - A `ChunkStream` gives its `max_in_flight` slot back at `End` or its error, not at drop.
  - A `Subscription` keeps up to 1024 unread events; later ones are dropped and counted by
    `Subscription::dropped() -> u64`.
  - WebSocket: an envelope whose zstd form is not smaller goes plain, so a message is never
    over the envelope plus its flag byte. `ws://`, `wss://` and the Unix socket share one limit
    (16 MiB + 1 MiB per message and frame); for `wss://` it comes through
    `tilr_websocket::connect_with_config`, added beside `connect`.
- **Test runtime** `examples/test_runtime.rs`: besides the methods of task 1.4 it provides
  `test.records` (what `test.record`, aborted sleeps, cancelled streams and received events
  recorded), `test.crash` (a panic: `handler_error`), `test.emit` and `test.subscribe`; it takes
  its client name from `TEST_CLIENT_NAME` (else `TILR_INSTANCE`), subscribes at start to
  `TEST_SUBSCRIBE`, and connects to the server's Unix socket with `TILR_BUS_URL=unix:<path>`.
  `test.whoami` answers the identity, or the client name for an `owner` connection.

## Reload and reconnect

Nothing of this change runs in the server. The client reconnects only when asked (the callers
decide, T10).

## Backpressure

The client keeps at most `max_in_flight` calls pending (later ones wait) and returns stream
credit only as the caller consumes chunks (B7); its outbox is bounded as `tilr.v1`'s client is
today (T5).
