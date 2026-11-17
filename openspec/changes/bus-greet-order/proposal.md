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
- A test-only failpoint `bus-greeted:<client name>` holds a connection right after its
  greeting, so tests can look at what the broker shows of it there.
- Two bus tests that raced the test itself are fixed: `an_instance_is_stopped_and_started`
  waits for its log line instead of reading the log once, and
  `events_reach_the_subscribers_connected_now` orders an event emitted on one connection
  before a subscription made on another.

## Capabilities

### Modified Capabilities
- `component-bus`: a greeted participant is callable and listed from the moment it has its
  `HelloAck` or anybody saw it up.

## Non-goals

- The rest of the broker and the transport (routing, drains, streams) is unchanged.

## Impact

`crates/tilrd/src/bus/broker.rs` and `crates/tilrd/src/bus/transport.rs` (the greeting),
`crates/tilrd/tests/bus.rs`. Protocol: version 1, no flag. No public API changes.
