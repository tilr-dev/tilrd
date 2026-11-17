# Proposal: A bus drain ends at its deadline

## Why

Draining a connection (an instance being stopped or replaced) and draining the whole broker
(a reload) wait for the calls in flight up to a timeout. The wait was woken by every release
of the broker's state lock, including the release of its own look at the state, so it woke
itself at once and never let the timeout win: with a call that is never answered, the drain
did not end and kept a CPU core busy.

## What Changes

- A drain looks at the broker's state without waking the waiters, so it sleeps until the
  state changes or its timeout passes.
- A drain with a call that is never answered ends at its timeout, answers the call
  `unavailable` and cancels it at the callee.

## Capabilities

### Modified Capabilities
- `component-bus`: a drain of a connection and of the broker ends at its deadline without
  spinning.

## Non-goals

- Other waits of the broker (stream credit, a reader catching up, the call timers) wait on
  their own notifications and are unchanged.

## Impact

`crates/tilrd/src/bus/broker.rs` only. Protocol: version 1, no flag. No public API changes.
