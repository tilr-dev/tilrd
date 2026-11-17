## ADDED Requirements

### Requirement: A greeted participant is callable
A greeted connection SHALL be entered as a connection and as the provider of the methods its `Hello` names, with its `HelloAck` queued as its first envelope, before it is announced up. Once the participant has its `HelloAck`, or anybody saw it up (`bus.start`, `bus.reload`, the `participant greeted` log line), its methods SHALL be routed to it, listed by `bus.methods` and known to the broker.

**Verified by**: `bus::a_participant_is_callable_once_it_has_its_hello_ack`, `bus::a_started_instance_is_callable_and_listed_when_bus_start_returns`.

#### Scenario: A call right after the HelloAck
- **WHEN** a participant providing `test.late` has its `HelloAck` and is held right after its greeting, and the owner calls `test.late`
- **THEN** the call reaches the participant instead of being answered `not_found`, and its answer reaches the owner

#### Scenario: An instance when bus.start answers
- **WHEN** `bus.start {"instance": "t-1"}` has answered and `t-1` is held right after its greeting
- **THEN** `bus.methods` lists `instance:t-1` as the provider of `test.echo`, a call of `test.echo` is answered by `t-1`, and once `t-1` is stopped a call of `test.echo` is `unavailable`
