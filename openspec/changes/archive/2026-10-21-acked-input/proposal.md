## Why

An input frame successfully sent to a socket can disappear when that connection ends. Retrying it without an acknowledgement can instead apply it twice.

## What Changes

- Number input per stable client identity and pane, acknowledge application independently of screen output, and replay unacknowledged input after reconnecting.
- Persist applied and accepted input sequences and outstanding byte boundaries across pane exec.
- Bound client queues and report overflow explicitly.
- Require the same no-loss behavior in the typing-through-update end-to-end case.

## Capabilities

### Modified Capabilities
- `protocol`: input delivery sequence, acknowledgements and handshake recovery in the existing version contract.
- `reload`: exactly-once application across connection loss and exec.
- `client-liveness`: fresh subscriptions with input replay and deduplication across connections.
- `tilr-cli`: retain input during reconnection and report bounded queue overflow.

## Impact

Protobuf schema, pane handover, supervisor routing, terminal client, driver and deterministic integration tests. The protocol version and pane state format remain 3; their contracts change in place.
