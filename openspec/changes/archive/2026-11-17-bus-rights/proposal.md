# Proposal: Biscuit rights on the bus

Зависит от: bus-instances

Стартует после: bus-broker-core (contract: `Authorizer`, `Rights`); мержится после: bus-instances

Владеет: `crates/tilrd/src/bus/acl.rs`, the body of `default_authorizer` in `bus/authz.rs`,
`crates/tilr-bus/src/credential.rs` (`Credential::attenuate`, additive, behind the `tilr-bus`
feature `attenuate`), the `biscuit-auth` dependency (`tilrd`, `tilr-bus`),
`tests/common/bus_rights.rs`, `crates/tilrd/tests/bus_acl.rs`, README "The bus" rights
subsection, `deny.toml` if a license must be allowed.

## Why

Owner's decisions of 2026-11-17 (`openspec/design/unified-bus.md` Part R,
O3): least privilege with Biscuit for every participant but the owner; bearer per-call
credentials; 24 h expiry with renewal; rotation ends every token; 1 ms limit with caches.

## What Changes

- Root key `<state dir>/bus-root.key` (0600, made once); tokens minted from `rights` for each
  instance start (`HelloAck.credential`), and by the owner with `bus.token` (with `origin` and
  `ttl` ≤ 24 h); `Hello.credential` for outside participants; TCP participants without the access
  token need a token (R2, R3).
- Checks for call, provide, emit, subscribe and control (R3, R6), `Call.credential` as a bearer
  (R4), attenuation (`Credential::attenuate` in `tilr-bus`), the `origin` fact (R6, W2).
- Expiry, renewal (`CredentialUpdate` both ways, `bus.renew`), revocation (`bus.revoke`,
  `<state dir>/bus-revoked`), rotation (`bus.rotate_key`), limits and caches (R5, R7), audit
  lines (R8).
- New close code **4003** `credential` (part of `tilr.bus.v1` version 1, B14; not breaking: no
  released client speaks the bus); documented in README "The bus" here and in README
  "Protocol" by `remove-tilr-v1`.

## Capabilities

### New Capabilities
- `bus-acl`: the root key, the owner, instance tokens; the rights to provide, emit, subscribe
  and control; server binding, bearer credentials, attenuation, expiry and renewal, `bus.renew`,
  revocation, rotation, bounded fail-closed authorization, audit. ("The owner mints tokens", the
  `tilr bus token` command, is `bus-tilr-client`'s.)

## Non-goals

- Pane rights (`bus-pane-rights`); the `tilr bus token|revoke|rotate-key` commands
  (`bus-tilr-client`); Origin on the handshake and page tokens (`bus-web`); Biscuit for the TCP
  access token, third-party blocks, proof of possession (shared design).

## Impact

`bus/acl.rs`, `bus/authz.rs`, `tilr-bus`, `Cargo.toml`/`Cargo.lock`, `deny.toml`, tests, a
cost measurement (an ignored unit test), README. New files in the state directory: `bus-root.key`, `bus-revoked` (0600).
Reload: tokens of instances do not cross a reload until `bus-reload` (instances are restarted
and get new tokens, `bus-instances`' interim); the root key and revocations persist in files.
Nothing released from `main` (O5).
