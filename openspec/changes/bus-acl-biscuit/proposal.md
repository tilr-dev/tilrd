# Proposal: Biscuit capability tokens for the bus

Зависит от: component-bus

## Why

On 2026-11-17 the owner decided (session TILR-892): access control on the bus is Biscuit
capability tokens (crate `biscuit-auth`): rights as Datalog facts, signed by a root key held by
tilrd, verified with its public key; a holder can attenuate a token without the root key; expiry
by a time check; the bus ACL is the first place Biscuit is used (not connection tokens, not pane
rights). Measured by the owner on Apple M2 Max, biscuit-auth 6.0.0, release: parse + verify +
authorize 54 µs (no extra block, 210 B), 95 µs (+1 block), 260 µs (+5 blocks); authorize alone
5.6-23 µs; with a cache of parsed tokens 7-29 µs; with a decision cache (TTL 1 s) 115-428 ns;
some authorizations hit the default 1 ms Datalog run limit under 12-thread load without the
decision cache. The prototype's bus round trip is about 70 µs in-process, so verifying every
message from scratch is not acceptable: parsed-token and decision caches, fail closed on the run
limit.

The prototype's ACL (`runtime-tmp` `broker.rs` `allowed`: instance-name globs to method globs)
has no delegation: the gateway `gw` calls for every HTTP client and may call `bus.*` and
`debug.*` (`debug.call` = act as anyone), so any local HTTP client can reload the bus
(`docs/how-it-works.md` §10). `component-bus` ships a minimal stand-in (`calls` globs, `owner`
may do everything); this change replaces it.

## What Changes

- tilrd keeps an Ed25519 root key in `<state dir>/bus-root.key` (0600, made on first start).
- Every instance gets a token minted by tilrd at its start from `[instances.<name>.rights]` in
  `bus.toml` (`call` method patterns, `control` actions on instances), delivered in
  `HelloAck.credential`, valid 24 h and renewed by the broker (`CredentialUpdate`) while the
  process lives; revoked when it ends.
- Rights model (Datalog facts): `instance`, `call` (exact method), `call_prefix` (namespace
  `ns.*`), `call_any`, `control(action, instance)`, `control_any(action)`, `mint`; every token is
  bound to the server (`check if server(<server id>)`) and expires (`check if time`).
- A call is authorized by `Call.credential` when present (bearer, e.g. a token an HTTP client
  gave the gateway), else by the connection's own token. Holders attenuate offline (append a
  block with `check if` on method, target, time).
- The owner (greeted full tilrd client) mints tokens with `tilr bus token` (`bus.token`), revokes
  with `tilr bus revoke` (`bus.revoke`), rotates the root key with `tilr bus rotate-key`
  (`bus.rotate_key`: every token dies, instances get new ones by a rolling reload).
- Broker verification: parsed-token cache (by token hash), decision cache (TTL 1 s, cleared on
  revocation, key rotation and config reload), revocation set checked on every call; Datalog run
  limits (1 ms, 1000 facts, 100 iterations), token size (16 KiB) and block count (8) limits; any
  limit hit = `denied` (fail closed).
- Audit log lines for every denied call, every control call and every mint/revoke.
- `calls` in `bus.toml` is replaced by `rights` (**BREAKING** for `bus.toml` files written for
  `component-bus`; migration: `calls = [...]` → `[instances.<n>.rights] call = [...]`; the old
  key is a config error naming the new one). Pre-release, no published users.
- Bus protocol: `Hello.credential` (unused by spawned instances), `Call.credential` (reserved in
  `component-bus`), `HelloAck.credential`, `Envelope.credential_update`; additive within
  `tilr.bus.v1`. tilr.proto unchanged except `BusCall.credential`; no feature flag.

## Capabilities

### New Capabilities
- `bus-acl`: tokens, rights, minting, attenuation, expiry, revocation, verification, limits,
  audit.

### Modified Capabilities
- `component-bus`: "Calls are denied unless allowed" (replaced by tokens). Written against the
  `component-bus` delta (archived first).

## Non-goals

- Biscuit for tilrd's client connections (the TCP access token) and for pane rights inside tilrd
  (later; `read-only-viewer` and `multi-user-sessions` keep their own designs).
- Third-party blocks; public-key-bound holders (proof of possession).
- A rights UI; per-payload (argument-level) rules beyond the method and target.
- Multiple root keys at once (rotation is all-or-nothing).

## Impact

`crates/tilr-bus` (`proto/bus.proto`, client: `credential()`, `attenuate()`, per-call
credential), `crates/tilrd/src/bus/{acl,broker,config,control}.rs` (new `acl.rs`), `deny.toml`
if `biscuit-auth`'s tree needs a license not yet allowed, `crates/tilr/src/` (`tilr bus token |
revoke | rotate-key`), `crates/tilr-proto/proto/tilr.proto` (`BusCall.credential`), README
("Components": rights), USE_CASES.md.

Reload: the root key and the revocation set are files (`bus-root.key`, `bus-revoked`); instance
tokens stay valid (same key, same processes); caches start empty. `bus-state` goes to 2: each
instance's token revocation ids and renewal time are handed over, so the successor can revoke
them when the instance ends. Update from the previous release: the key is
made on first start.
