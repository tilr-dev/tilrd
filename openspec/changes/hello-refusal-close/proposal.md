# Handle control frames while waiting for refusal closure

## Why

A handshake-refusal test treats the next WebSocket frame after the protocol Error as the Close. The supervisor queues the Error before signalling closure to its independent writer, which can send a normal liveness Ping between those steps. The connection still closes correctly.

## What Changes

- Document that WebSocket control frames may precede the refusal Close.
- Force the Error → Ping → Close ordering with a test-only pause after the refusal Error is queued.
- Have the test accept Ping/Pong frames while still requiring the refusal Close and rejecting additional application data or a liveness-timeout Close under its existing deadline.

## Capabilities

### Added Capabilities
- `protocol`: an additive requirement for observing handshake-refusal closure with control frames.

## Impact

The input-delivery integration test, a failpoint in the supervisor refusal path, and its use-case description. No wire-format or production-behavior change.
