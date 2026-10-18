# Proposal: Feature flags for the additive protocol changes of the itests branch

Блокер: Flags or PROTOCOL_VERSION 3 for the itests-branch protocol changes? Options: (a) add features `path-bytes`, `client-id`, `key-code`, `bulk-pong` to HelloAck — old clients keep working, new clients can detect old servers; (b) bump PROTOCOL_VERSION to 3 — simpler rule, every client must update with the server; (c) leave as is. Recommendation: (a), since the README's rule is "grow by adding and advertise in features" and clients are not all released together.

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (protocol section): all changes on the itests branch are additive and PROTOCOL_VERSION is still 2, with only `exec` advertised; there are no flags for `Hello.client_id`, `KeyInput.code`, the `*_bytes` twins, `PaneInfo.viewed_by` / `used_at_ms` or close codes 4000-4002. A new client cannot tell that an old server ignores `path_bytes`, so a lossy name hits the wrong file. Old clients are now dropped when they do not pong (4002) and bulk is paced by pongs.

## What Changes

- To be decided: either `HelloAck.features` gains `path-bytes`, `client-id`, `key-code` (and `bulk-pong` for pong pacing), and clients use the twins only when `path-bytes` is present; or PROTOCOL_VERSION goes to 3 and v2 clients are refused.

## Capabilities

### Modified Capabilities
- `protocol`
- `files`
- `client-liveness`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Changing the close codes themselves.

## Impact

`crates/tilr-proto` (README "Protocol"), `crates/tilrd/src/supervisor.rs`, clients (Tilr app `server` plugin, tilr-driver, `tilr`). Flags: not breaking. A version bump: **BREAKING** for every client (migration: update clients with the server). Survives a reload either way (features are per connection).
