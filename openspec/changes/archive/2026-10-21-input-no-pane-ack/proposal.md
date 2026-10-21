## Why

At commit c2ae751, the supervisor returns only an Error for input naming no running pane. The client outbox retains numbered input until InputAck, so rejected entries are replayed on every reconnect and keep consuming bounded queue capacity.

## What changes

- Add a terminal rejection acknowledgement for numbered input naming no running pane, preserving the existing error.
- Reuse InputAck without changing wire fields, protocol version 1 or feature flags. This is a mandatory pre-release protocol clarification.
- No new state crosses reload. A replay after reconnect or reload receives the same terminal rejection while the pane is absent.

## Out of scope

- Changing applied counters, live-pane delivery, outbox limits or other request errors.
- Persisting rejection history or client queues across process restarts.
