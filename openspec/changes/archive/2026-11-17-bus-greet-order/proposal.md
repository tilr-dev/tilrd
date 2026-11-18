# Proposal: A bus connection is registered before it is announced

## Why

The broker greeted a connection in the wrong order: it queued the `HelloAck`, logged
`participant greeted` and announced the connection up, and only then entered it as a
connection and as the provider of its methods. In that window a participant holding its
`HelloAck` (or anybody it told) got `not_found: no participant provides it` for a call of its
own method, and the instance runner, woken by the announcement, read the connections before
the instance was among them: `bus.start` and `bus.reload` answered while the instance's
methods were neither callable nor listed, and the methods it provides were not remembered, so
a call while it was down later answered `not_found` instead of `unavailable`. Tests that wait
for the `participant greeted` line and call at once failed now and then for the same reason.

## What Changes

- A greeted connection is entered as a connection and as the provider of its methods, and
  its `HelloAck` is queued, under one hold of the broker's lock; the `HelloAck` stays the
  first envelope it gets. Only then is it announced up and `participant greeted` logged.
- An instance being stopped or reloaded is marked draining in the broker before its state
  turns `draining`, so whoever sees it `draining` in `bus.instances` sees it offered no new
  calls by method (`bus.methods` lists it no longer), as the main spec's "Draining is not offered"
  already says; `participants_and_methods_are_listed` saw the two apart now and then.
- Test-only failpoints: `bus-registering:<client name>` holds a greeted connection just
  before the broker enters it, `bus-draining:<instance>` holds an instance between the
  broker's drain and its state `draining`, so tests can look at what is observable there.
- Two bus tests that raced the test itself are fixed: `an_instance_is_stopped_and_started`
  waits for its log line instead of reading the log once, and
  `events_reach_the_subscribers_connected_now` orders an event emitted on one connection
  before a subscription made on another.

## Capabilities

### Modified Capabilities
- `component-bus`: adds the requirement "A greeted participant is callable": a greeted
  participant is callable and listed from the moment it has its `HelloAck` or anybody saw it
  up; and the requirement "A draining instance is offered no calls before it is listed
  draining". No existing requirement changes.

## Non-goals

- The rest of the broker and the transport (routing, drains, streams) is unchanged.

## Impact

`crates/tilrd/src/bus/broker.rs` and `crates/tilrd/src/bus/transport.rs` (the greeting),
`crates/tilrd/src/bus/runner.rs` (the order of an instance's drain),
`crates/tilrd/tests/bus.rs`. Protocol: version 1, no flag. No public API changes.
