## ADDED Requirements

### Requirement: A drain ends at its deadline
Draining a connection, and draining the broker, SHALL wait for the calls in flight until they are answered or the drain's timeout passes, whichever comes first, woken only when the broker's state changes (no polling, no busy loop). At the timeout, the calls still in flight SHALL be answered `unavailable` and their callees SHALL get `Cancel`.

**Verified by**: unit tests `draining_a_connection_ends_at_its_deadline_without_spinning`, `the_broker_drain_ends_at_its_deadline_without_spinning` (crates/tilrd/src/bus/broker.rs).

#### Scenario: A provider that never answers is drained
- **WHEN** a call to `p-1` is in flight, `p-1` never answers it, and `p-1`'s connection is drained with a timeout of 50 ms
- **THEN** the drain returns after 50 ms reporting the call unanswered, having looked at the state only a few times; the caller gets `unavailable` and `p-1` gets `Cancel`

#### Scenario: The broker is drained with a call in flight
- **WHEN** a call to `p-1` is in flight, `p-1` never answers it, and the broker is drained with a timeout of 50 ms
- **THEN** the drain returns after 50 ms, having looked at the state only a few times; the caller gets `unavailable` and `p-1` gets `Cancel`
