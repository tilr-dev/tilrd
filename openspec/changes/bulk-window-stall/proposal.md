# Proposal: The bulk window cannot stall; a lagging client gets its last frame

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found (findings #5, #13, #17): the bulk window can stall for ever when a client answers only the latest ping, because an empty liveness ping can follow the ping that carried the position (`supervisor.rs:1048`, `outbox.rs:309-333`; tungstenite behaviour taken from memory, untested); a lagging client's final full frame can miss its wake-up (`supervisor.rs:2316-2326` vs `outbox.rs:258-259`); a client can widen its window by lying in pongs (bounded by 4 MiB) (`outbox.rs:331`).

## What Changes

- While the bulk window is closed every ping carries the current position, so any pong reopens it.
- Queuing the full frame of a lagging client always wakes its writer.
- A pong's position is capped at the bytes actually sent.

## Capabilities

### Modified Capabilities
- `client-liveness`: see `specs/client-liveness/spec.md`.

## Out of scope

- Changing the window sizes (4 MiB per client, 1 MiB unread).
- QUIC transport.

## Impact

`crates/tilrd/src/{outbox,supervisor}.rs`. Nothing crosses a reload (the outbox is per connection); no protocol change; clients that answer pings as now keep working.
