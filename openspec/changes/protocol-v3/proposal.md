# Proposal: Protocol version 3: the itests-branch additions become the mandatory contract

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (protocol section): all changes on the itests branch are additive and PROTOCOL_VERSION is still 2, with only `exec` advertised; there are no flags for `Hello.client_id`, `KeyInput.code`, the `*_bytes` twins, `PaneInfo.viewed_by` / `used_at_ms` or close codes 4000-4002. A new client cannot tell that an old server ignores `path_bytes`, so a lossy name hits the wrong file. Old clients are now dropped when they do not pong (4002) and bulk is paced by pongs.

## Decision (2026-10-18)

- PROTOCOL_VERSION becomes 3. From now on PROTOCOL_VERSION is the mandatory wire contract: any change a peer cannot ignore bumps it, and a server refuses every other version.
- `HelloAck.features` lists only capabilities a server can be run without (today `clipboard`, `files`, `host-events`, `agent`, `exec`); `ping` is always on, so it leaves the list.
- Everything added on the itests branch is mandatory in v3, with no flags; no compatibility with older clients or servers (no client has been released).
- Rejected: feature flags `path-bytes` / `client-id` / `key-code` / `bulk-pong` (flags would stand for things no server can switch off).
- Rejected: leaving version 2 as is (a client cannot tell an old server from a new one).

## What Changes

- **BREAKING**: `PROTOCOL_VERSION` 2 → 3 (`crates/tilr-proto/src/lib.rs`); a v2 client is refused with `unsupported protocol version 2: this server speaks 3`, and a v3 client gets the same from a v2 server. Migration: the clients (`tilr`, `tilr-driver`, the Tilr app's `server` plugin, tilr-mac) update together with the server. A v3 server and client implement `Hello.client_id`, `KeyInput.code`, the `*_bytes` path twins, pong-paced bulk, `PaneInfo.viewed_by` / `used_at_ms` and close codes 4000-4002.
- **BREAKING**: `"ping"` is no longer listed in `HelloAck.features`; `Ping` is part of v3.
- The rule for later changes, in README "Protocol", the proto header comment and `openspec/config.yaml`: a change a peer must implement bumps PROTOCOL_VERSION; a feature names a capability the server can be run without.
- Open questions (design.md): whether the string paths go and only `*_bytes` stay; whether the server refuses an empty `client_id` / `code`.

## Capabilities

### Modified Capabilities
- `protocol`: version 3; the rule for `HelloAck.features`; what v3 makes mandatory.
- `screen-frames`: a `KeyInput` without `code` is a key with no physical position, not an older client.

## Non-goals

- Changing the close codes, the `*_bytes` fields or any message besides `PROTOCOL_VERSION` and the features list.
- Compatibility with v2 clients or servers.
- Client-side changes outside this repository (the `server` plugin, tilr-mac), beyond saying they update together.

## Impact

`crates/tilr-proto` (`PROTOCOL_VERSION`, proto comments), `crates/tilrd/src/supervisor.rs` (features list), `crates/tilr` and `crates/tilr-driver`, README "Protocol", `openspec/config.yaml`. Nothing crosses a reload (version and features are per connection); clients reconnect after the reload into the v3 server and must speak v3. An update from the previous release works for the server (no state format change); its v2 clients are refused until they update.
