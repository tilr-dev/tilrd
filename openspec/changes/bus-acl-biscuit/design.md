# Design: Biscuit capability tokens for the bus

## Context

`component-bus` gives every connection an identity (the instance it was made for, or `owner` for
tilrd's full clients), a minimal rule (`calls` globs; `owner` everything) checked before routing,
the reserved fields `Hello.credential` and `Call.credential`, and control methods `bus.*`. The
owner's measurements (proposal) set the performance frame: from-scratch verification costs about
as much as a bus round trip, a decision cache brings it under 0.5 µs.

Biscuit (biscuit-auth 6): an authority block signed by the root key, appended blocks signed by
an ephemeral key chain (attenuation without the root key, offline), Datalog facts/rules/checks
per block, an authorizer with its own facts and `allow`/`deny` policies, revocation ids per block,
run limits (`max_time` default 1 ms, `max_facts` 1000, `max_iterations` 100).

## Goals / Non-goals

Goals: least privilege for every instance and every outside caller; delegation by attenuation;
expiry and revocation; verification cheap enough per call; fail closed.

Non-goals: see proposal.

## Decisions

### D1. Resources and actions
Authorizer facts for each call (supplied by the broker, never by the token):
`caller(<connection identity>)`, `method(<method>)`, `namespace(<first segment>)`,
`target(<instance or "">)`, `server(<server id>)`, `time(<now>)`, and for control methods
`action(<start|stop|reload|reload_config|list|token|revoke|rotate_key>)`, `subject(<instance or
"*">)`.

Rights (facts in the authority block, minted by tilrd):
- `call("math.add")` — one method; `call_prefix("math.")` — every method of a namespace (config
  `math.*`); `call_any()` — every non-`bus.*` method (config `*`).
- `control("reload", "t-1")`, `control_any("reload")` — a control action on one instance or any;
  `list` covers `bus.instances` and `bus.methods`; `reload_config`, `rotate_key` take no subject.
- `mint()` — may call `bus.token` (mint new tokens); `revoke()` — `bus.revoke`.

Authorizer policies: `allow if method($m), call($m)`; `allow if method($m), call_prefix($p),
$m.starts_with($p)`; `allow if method($m), call_any(), !$m.starts_with("bus.")`; `allow if
action($a), subject($s), control($a, $s)`; `allow if action($a), control_any($a)`; the same for
`mint`/`revoke`; then `deny if true`. Rejected: globs as strings inside Datalog (`*` matching
needs a regex; `starts_with` is exact and cheap); per-instance ACL tables outside tokens (no
delegation).

### D2. Every token is bound and expires
Authority block always carries `check if server(<server id>)` (a token from another machine's
tilrd is useless here) and `check if time($t), $t <= <expiry>`. Instance tokens: 24 h, renewed by
the broker after 12 h with `CredentialUpdate{credential}` on the instance's connection. Minted
tokens (`bus.token`): `ttl` required, default 1 h, at most 30 days. Owner principal: a token
minted per tilrd start with `call_any`, `control_any` for every action, `mint`, `revoke`, kept in
memory only (the owner is identified by the tilrd connection, not by a bearer).

### D3. Minting from config
```toml
[instances.gw.rights]
call = ["term.list", "math.*"]          # exact, ns.*, or "*"
control = ["list", "reload:gw"]         # action or action:instance-glob ("*" any)
```
At each process start tilrd mints the token (`instance("gw")` + rights + D2 checks) and sends it
in `HelloAck.credential`. No rights = a token with no `call`, so every call is denied (default
deny). `component-bus`'s `calls` key is a config error: `calls was replaced by [instances.<n>.rights] call`.

### D4. How a token travels
- At connect: spawned instances do not present one (identity is the fd); tilrd binds the minted
  token to the connection. `Hello.credential` is reserved for a future transport where the
  connector is not spawned by tilrd; it is ignored for spawned instances.
- Per call: `Call.credential` (and `BusCall.credential` on the client protocol) — when non-empty,
  the call is authorized by it alone (bearer), the connection's token is not consulted; `from`
  stays the connection's identity, the audit line names both. This is how the gateway forwards
  an HTTP caller's rights (`bus-gateway`) and how an instance passes a narrowed token to another.
- Nested calls do not inherit the incoming call's credential automatically; an SDK may forward
  it on request (`ctx.call(…, {credential: ctx.credential})`).

### D5. Attenuation
A holder appends a block with checks only (`check if method($m), ["term.read","term.write"]
.contains($m)`, `check if target("term-1")`, `check if time($t), $t <= …`) using the client
library (`Credential::attenuate`), without contacting tilrd. Facts in appended blocks are not
trusted by the authorizer policies (biscuit-auth scopes authority facts by default), so a block
can only narrow. At most 8 blocks per token; at most 16 KiB.

### D6. Revocation
Each token's revocation ids are checked against a set held by the broker. The set gets: the ids
of an instance's token when its process ends or its token is renewed; ids named by `bus.revoke
{token | revocation_id}`. Entries are kept until the token's expiry passes, in memory and in
`<state dir>/bus-revoked` (0600, written whole, read at start), so a revoked minted token stays
revoked across restarts. `bus.rotate_key` makes a new root key: every token dies at once, the
owner token is re-minted, instances are reloaded one by one to get new tokens (drained as
`bus.reload all`). Rejected: short-lived tokens without revocation (minted tokens of 30 days need
a kill switch); a key-id overlap window (owner's call; open question).

### D7. Verification in the broker
For each call: (1) token bytes → SHA-256 → parsed-token cache (LRU 1024 entries; a miss parses
and verifies the signature chain; an entry dies with the token's expiry or a key rotation);
(2) revocation ids of the parsed token against the set (every call, also on a decision-cache
hit); (3) decision cache keyed by (token hash, caller, method, target, action, subject) → allow
or deny, TTL 1 s, cleared whole on revocation, key rotation and `bus.reload_config`; (4) on a
miss, a fresh authorizer with D1's facts and policies, run with limits `max_time` 1 ms,
`max_facts` 1000, `max_iterations` 100. Any error — parse, signature, expired, revoked, limit
reached, no policy matched — is `denied` with a short reason (`expired`, `revoked`,
`authorization limit reached`, `not allowed`). Never allow on error. Time checks with a 1 s
decision TTL: a token may be used up to 1 s past its expiry (documented).

### D8. The gateway is not root
The prototype's `gw` had `bus.*`, `debug.*`, `term.*` for all callers. Here the gateway instance's
own rights are empty by default; it forwards each HTTP caller's token as `Call.credential`
(D4). A request without a token is refused by the gateway before it reaches the bus
(`bus-gateway`). There is no `debug.call` "act as" method in the product.

### D9. Audit
Log lines (target `bus::audit`, info level): every denied call `bus audit: deny from=<caller>
token=<first revocation id, 16 hex> method=<m> target=<t> reason=<r>`; every control method and
`bus.token`/`bus.revoke`/`bus.rotate_key` call, allowed or not, with the minted rights and ttl;
allowed ordinary calls only at debug level. No token bytes in logs.

### D10. Owner and other tilrd clients
`owner` = a greeted tilrd connection with full access, as in `component-bus`. A future read-only
connection (`read-only-viewer`) gets no bus access (its `BusCall` is refused by that change's
allow list). In a system tilrd (`multi-user-sessions`), each user would be its own `owner` with
a per-user broker or a `user(<uid>)` fact in every token; not designed here.

## Reload and reconnect

tilrd reload: the root key and revocation file are re-read; instance token ids and renewal times
are handed over in `bus-state` 2; the caches start empty (first calls after reload pay the parse
cost). A tilrd client reconnecting gets the owner identity again. An instance reload gets a new
token; the old one is revoked.

## Backpressure

No new data in quantity. Verification cost is bounded per call by D5's size/block limits and
D7's run limits; the decision cache keeps the common path under a microsecond.

## Risks / Trade-offs

- Up to 1 s of use after expiry (decision TTL) and after a revocation is not possible (revocation
  checked every call).
- Bearer tokens can be stolen from a process of the same user; same-user processes can already
  read the state directory. Proof of possession is a non-goal.
- `biscuit-auth`'s dependency tree must pass `cargo deny` and the MSRV.

## Open Questions

- D6 key rotation without overlap (every token dies). Rec.: all-or-nothing now (simplest; NATS
  operator key rotation also re-issues); add `root_key_id` overlap if a deployment needs it.
- D7 run limit 1 ms (biscuit default) vs higher. Rec.: keep 1 ms with the decision cache; raise
  only if `authorization limit reached` appears in real logs.
- D2 instance tokens 24 h with renewal vs no expiry plus revocation. Rec.: 24 h + renewal
  (short-lived credentials, as SPIFFE/SVIDs do).
- D4 bearer semantics for `Call.credential` (the connection's own token ignored). Rec.: bearer;
  intersecting both would make a gateway need every right of every caller.
- D3 the `calls` → `rights` rename breaking pre-release `bus.toml`. Rec.: accept (no users yet).
