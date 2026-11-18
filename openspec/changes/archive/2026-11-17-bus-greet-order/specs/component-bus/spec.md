## ADDED Requirements

### Requirement: A greeted participant is callable
A greeted connection SHALL be entered as a connection and as the provider of the methods its `Hello` names, with its `HelloAck` queued as its first envelope, before it is announced up. Once the participant has its `HelloAck`, or anybody saw it up (`bus.start`, `bus.reload`, the `participant greeted` log line), its methods SHALL be routed to it, listed by `bus.methods` and known to the broker.

**Verified by**: `bus::a_participant_is_callable_once_it_has_its_hello_ack`, `bus::a_started_instance_is_callable_and_listed_when_bus_start_returns`.

#### Scenario: Nothing of a participant before it is entered
- **WHEN** a participant providing `test.late` said `Hello` and is held just before the broker enters it
- **THEN** it has no `HelloAck` and is not logged `participant greeted`, while another participant greets and answers a call of the owner

#### Scenario: A call right after the HelloAck
- **WHEN** a participant providing `test.late` has its `HelloAck`, and the owner calls `test.late`
- **THEN** the call reaches the participant instead of being answered `not_found`, and its answer reaches the owner

#### Scenario: An instance when bus.start answers
- **WHEN** `bus.start {"instance": "t-1"}` is called and `t-1` is held just before the broker enters it
- **THEN** `bus.start` has not answered and `t-1` is `starting`, while another participant greets and answers a call of the owner; once `bus.start` answers, `bus.methods` lists `instance:t-1` as the provider of `test.echo`, a call of `test.echo` is answered by `t-1`, and once `t-1` is stopped a call of `test.echo` is `unavailable`

### Requirement: A draining instance is offered no calls before it is listed draining
An instance being stopped or reloaded SHALL be offered no new calls by method before `bus.instances` lists it `draining`: whoever sees it `draining` sees a call of its methods not routed to it.

**Verified by**: `bus::a_draining_instance_is_offered_no_calls_before_it_is_listed_draining`.

#### Scenario: A call while the instance is not yet listed draining
- **WHEN** `bus.stop {"instance": "t-1"}` is called and `t-1` is held after the broker stopped offering it calls, before it is listed `draining`
- **THEN** a call of `test.echo` is `unavailable`, `bus.methods` does not list `instance:t-1` as its provider, and `bus.stop` answers once `t-1` is released
