# Design: Biscuit rights on the bus

Shared design: `openspec/design/unified-bus.md` Part R (R1-R9), §B2, §W2 (origin as a fact).
This change's part follows.

## Decisions

- **One swap point.** The Biscuit authorizer implements the `Authorizer` trait of
  `bus-broker-core`'s contract; `default_authorizer` returns it; no caller changes. Instances
  get a token minted from their `Rights` at each start, through `hello_ack_credential`.
- **Pane facts are minted, not checked, here.** `pane = [...]` rights become the facts of R3 in
  the token; the core's second authorization (T7) is `bus-pane-rights`'s.
- **Commands later.** Tests mint with `bus.token`, revoke with `bus.revoke` and rotate with
  `bus.rotate_key` directly; `tilr bus token|revoke|rotate-key` (named in the scenarios) are
  `bus-tilr-client`'s.
- **Benchmark.** tilrd is a binary without a library target, so a criterion benchmark in
  `benches/` cannot reach the authorizer; the measurement is the ignored unit test
  `supervisor::bus::acl::tests::authorize_cost` (no `criterion` dependency), and its numbers
  go in the PR; a cached decision under 1 µs on CI hardware is a note, not a gate.
- **Broker files untouched** *(coordinator's decision after the review of the broker)*: this
  change does not edit `bus/broker.rs` or `bus/transport.rs` (after `bus-broker-core` only
  `bus-streams-events`, then `bus-reload`, do); it uses the broker's API: `Authorizer::credential_update`
  for `CredentialUpdate` from a participant, `BrokerHandle::update_principal` for a renewed
  token, `BrokerHandle::send_credential_update` to send an instance its renewed or rotated
  token, `close_connection` to end a connection with close code 4003 on revocation or rotation.
- **`list` and `reload_config` over every instance.** The call needs the action over every
  instance (`list`, `reload_config`, `*`), also while no instance is configured: a right over
  some instances (`list:t-*`, `*:t-1`) does not reach `bus.instances`, `bus.methods` or
  `bus.reload_config` (the listings show every participant, the file can add any instance).
  `bus-instances`' methods still check each instance they read or act on, so a holder who
  narrows its token with an appended block (`reject if instance($i),
  !$i.starts_with("s-");`) lists or reconfigures `s-*` only (each of those checks has the
  call's facts too, so a check satisfied by `namespace("bus")` would not narrow them). `bus.start`, `bus.stop` and
  `bus.reload` keep the gate on some instance (`control_gate`).
- **`bus.reload_config` drops the decision cache**: `bus-instances`' method calls
  `acl::config_reloaded` once the file is applied (one line in `control.rs`).

## Reload and reconnect

Until `bus-reload`, a reload restarts instances (`bus-instances`' interim) and they get new
tokens; the old ones are revoked when their process ends. Outside participants reconnect with
their token. `bus-reload` hands over revocation ids and renewal times (B13).

## Backpressure

Authorization cost is bounded by R7: 1000 facts and 100 iterations per decision (the same
work on every machine), token ≤ 16 KiB and ≤ 8 blocks (and `mint` never makes a bigger one,
nor one of over 512 rights facts), parsed-token LRU 1024, decision cache TTL 1 s. The wall
clock is a backstop of 50 ms, not R7's 1 ms: under load 1 ms ended ordinary decisions (the
owner's measurement of R7 saw it under 12-thread load), and a denial the clock caused is the
machine's state, not the token's, so it is never cached; the deterministic limits' denials are.
Biscuit 6 looks at the wall clock only between fixpoint iterations and after each check, so
one costly check runs to its end (a block of 40 facts and a 5-way join in one check ran
87 s); the limits alone do not bound a decision. So an appended block holds checks only (no
fact, no rule), at most 16 (each `or` alternative counted), each of at most 16 predicates,
at most 64 expression operations in the block, without `.all`/`.any` (closures over
values), `.matches` or extern functions, and its predicates are the request's facts only
(`acl::REQUEST_FACTS`, one fact each per request, never the authority block's rights): a
check binds once and costs its expression, whatever it joins. `verify` checks this once per
token, so such a token is refused before any Datalog runs and is never parsed into the cache.
It reads each appended block from its signed bytes, the protobuf `schema::Block` decoded with
biscuit-auth's own prost and converted with biscuit-auth's `format::convert` and the token's
symbol table, exactly what the authorizer runs, never from its printed source: Biscuit 6
prints symbols unescaped and its parser skips comments, so a fact named
`check if kind("bus");/*f` and a predicate named `X*/check if kind` print 30 facts and a
5-way join as two checks on `kind` (15 s per decision). A block with a field the schema does
not know (a block must be its own canonical encoding byte for byte, so an unknown field
cannot stand in for a missing required one of the same size), a schema version above
biscuit-auth's, an unknown check kind or operation, a `trusting` scope (block or rule) or a
public key is refused too.
The decision cache is keyed by a SHA-256 of the token's hash and the request's
length-prefixed parts, so an entry is the same size whatever a call names; a `Call.target`
over 512 bytes (a participant's name is far shorter) is denied before it is decided, cached
or logged, and audit lines clip a target to 128 bytes.
`bus.renew` is accepted in the second half of a token's life only, so a holder cannot make the
server rewrite and fsync the revocation file at will, and a revocation drops the cached
decisions of the tokens it ends only.

## Deviations in the implementation

What the code froze where it differs from the decisions above or the shared design; the code
is the contract.

- **Datalog names.** A body predicate needs a term in Biscuit, so the "any" rights are
  `call_any(true)`, `provide_any(true)`, `emit_any(true)`, `subscribe_any(true)` and
  `control_all(true)` (`control = ["*"]`). Rights facts that would share a name with a request
  fact are `provide_method(m)`, `emit_subject(s)`, `subscribe_pattern(p)`; a control right on
  an instance prefix is `control_prefix(a, p)`. The request facts are `kind(k)` (`call`,
  `control`, `control_gate`, `provide`, `emit`, `subscribe`, `event`, `pane`, `bus`),
  `caller`, `method`, `namespace`, `target`, `subject`, `pattern`, `action`, `instance`,
  `scope`, `server`, `origin`, `time`. The policies are `acl::POLICIES`.
- **Metadata in the signed context.** The authority block's context (signed with it) holds
  the participant, the server id, mint and expiry times, the bound origin, the rights and, for a token a
  non-owner minted, its lineage (ceiling and minters' revocation ids) as JSON; the server
  reads them without Datalog (server id, expiry, origin, renewal, a non-owner's mint limits,
  revocation of a minter, the chain of renewals). The same facts and checks are in the block for Datalog (not the lineage,
  which only the server checks).
- **Control methods at the call.** `bus.instances`/`bus.methods` need `list`;
  `bus.start`/`bus.stop`/`bus.reload` pass the call when the token has that action on some
  instance (`control_gate`), and the method checks each instance it acts on (see "The
  control contract"); `bus.reload_config`, `bus.token`, `bus.revoke`, `bus.rotate_key` need
  their action; `server.reload` needs the action `server.reload` and `server.update` the
  action `server.update` (owner's decisions: actions of their own, not the instances'
  `reload`; `call = ["*"]` or `["server.*"]` covers neither). `bus.ping` and `bus.renew` need a
  valid token only. Only these exact names are control methods: any other `bus.*` method
  (`bus.absent`, a look-alike such as `bus.Token`) is a plain call, allowed to whom `call`
  rights cover it (the owner, `*`, `bus.*`) and answered `not_found` by the broker, and
  `denied` to others.
- **`server.command` is no right** (owner's decision): it is allowed to tilrd and to the owner
  on the Unix socket only, never on TCP (with the access token or `--no-auth`), never for a
  token (the connection's or per call), whatever `call` or `control` it lists. The handler of
  `bus-core-services` must check the same itself (owner and `Transport::UnixSocket`), so the
  rule holds should the authorizer change.
- **Minting by a non-owner.** A participant with `control = ["token"]` mints only rights it
  holds itself (verbatim, or under its `*` of the same kind), and only with a token without
  appended blocks (as `bus.renew`: the narrowing would not reach the child). The child is named
  `<minter kind>.<minter name>.<name>` (owner's decision, as Kubernetes names a service
  account `system:serviceaccount:<ns>:<name>`: `token:app` mints `token:token.app.child`,
  `instance:app` mints `token:instance.app.child`; the name rule of `bus.token` stays
  `[A-Za-z0-9_.:-]`, up to 128, the prefix included), so it cannot pass for another
  participant, nor for the child of a minter of the other kind with the same name. The
  child's own `<name>` has no dot (owner's decision, as Kubernetes forbids `:` inside the
  parts of `system:serviceaccount:<ns>:<name>`): instance names may hold dots, so without it
  `instance:a` could mint `instance.a.b.x`, the name of the child `x` of `instance:a.b`; with
  it the last component is the child and the rest its minter. Instance names are unchanged.
  The owner, who names any token, may not take the prefixes `instance.` and `token.` (owner's
  decision, as Kubernetes reserves `system:`): a token of the owner named `instance.a.x`
  would share the child's identity, by which the broker routes calls and supersedes
  connections.
  Revocation does not read names (a child's minters are revocation ids in its signed
  metadata), so the rule keeps names unambiguous for people and for what routes by name. A
  minter
  bound to an origin mints for that origin only (an empty `origin` takes it, another is
  denied). The child's signed metadata holds a ceiling (the minter's expiry: no renewal goes
  past it) and its minters' families (below): revoking a minter ends its children, as it ends
  tokens attenuated from it; renewing it (an instance's every 12 h) does not.
- **Chains of renewals (families).** A renewed token's signed metadata names its family: the
  authority revocation id of the first token of its chain, kept through every renewal (an
  instance's too). The revocation set ends a token whose blocks' ids, family or minters'
  families it holds. A renewal does not revoke the old token's id (for the first token that
  is the family): it marks it renewed, a second set (`<id> <until> renewed` in
  `bus-revoked`) that ends that token and those attenuated from it only. `bus.revoke` of a
  token without appended blocks revokes its family (kept a day from now: every token of the
  chain expires before), so a token renewed by a thief dies with the leaked one;
  `RenewReply.revocation_id` is the family's id, the one `bus.token` answered, so the id the
  holder has always ends the chain. An instance's family is revoked when its connection ends.
  Revoking the authority id of a later token of a chain (read from its bytes, never
  answered by the server) ends that token and what is attenuated from it only.
- **Per-call credential and follow-up checks.** A non-empty `Call.credential` decides the
  call and every check its method makes with it: each `AuthzRequest::Control` of a control
  method and the pane checks (`PaneCreate`, `PaneAccess`) of a `pane.*` call; it is refused
  for anything else (provide, emit, subscribe, events). Each such follow-up check carries
  the call's `method`, `namespace` and `target` (`ForCall`) besides its own facts, and its
  decision cache key holds them, so an appended check such as `check if
  method("pane.list")` decides the pane checks of `pane.list` as it decides the call, and a
  cached allow for one call is never reused for another. Audit lines of such a decision name
  `from=<caller> bearer=<the token's participant>`.
- **The revocation file** is written whole (staged under a name unique to the process, a
  counter and the time, fsynced, renamed, the directory fsynced) by one writer at a time: a
  lock is held from the snapshot of the set to the rename, so concurrent revocations all reach
  the file. A file that did not read at start is never written (every token stays refused,
  also after a restart) until the operator fixes or removes it. A rotation writes the key file
  the same way while holding the keys' lock, so the key in memory and on disk never differ; it
  keeps the revoked ids until they expire (clearing them could drop a revocation of a token of
  the new key made meanwhile).
- **File I/O off the envelope path.** The bodies of `bus.revoke`, `bus.renew` and
  `bus.rotate_key` (and `bus.token`) run on a blocking thread; the reply follows the body, so a
  token is revoked in memory and on disk before the caller hears back. An instance's token
  revoked from the envelope path or the sweeper is revoked in memory at once and written from
  a blocking thread.
- **No state directory** (no home): the root key lives in memory until the server stops, as
  the server id does then; revocations too.
- **Instances.** The token is minted in `Authorizer::hello` from the seat's rights and kept in
  the principal. It is revoked when the instance's connection ends: at once when another
  connection presents it (`BrokerHandle::principal_of`; the authorizer is never called under
  the broker's lock), otherwise by the sweeper (every 200 ms). Renewal after 12 h and the new
  tokens after a rotation replace the connection's principal (`update_principal`): the
  instance's own requests go on being allowed, and the instance gets the new token in a
  `CredentialUpdate` (`BrokerHandle::send_credential_update`, after the principal is
  replaced). A request from the instance's own connection marks it open at once. A seat not
  yet seen open is taken as not registered yet for 1 s after its `Hello` (the broker
  registers the connection right after it queued `HelloAck`); after that, its token presented
  on another connection while its own is not open is denied (the sweeper revokes it), and so
  is one presented on another connection while no broker is registered to ask (fail closed). The sweeper also renews an instance, or closes an outside participant with
  4003, whose token is of an earlier key: a connection that said `Hello` while a rotation ran.
- **Forged core events** are dropped by the broker before the authorizer; their audit line is
  the broker's (`dropped an event in a namespace of tilrd`) on the audit target, not
  `bus audit: deny`.
- **Pane requests** (`PaneCreate`, `PaneAccess`) are allowed for any valid token, as the stub
  did; their rules (T7) are `bus-pane-rights`'s, which reads the token from
  `acl::token_of(principal)`.
- **Event delivery** (`AuthzRequest::Event`, one per subscriber) is allowed for any valid
  token (`allow if kind("event")`; an appended block's checks still apply), also for an event
  with a `scope` (`pane:<id>`): who may receive a pane-scoped event (`pane_access`, shared
  design "pane-scoped events") is `bus-pane-rights`'s, handed over with the pane rules.
- **Dependency.** `biscuit-auth` 6.0.0 builds only with its `datalog-macro` feature; it brings
  two advisories ignored in `deny.toml` with their reasons (crypto-bigint 0.5 through p256,
  which tilrd never signs with; proc-macro-error2, build time only). biscuit-auth 7 drops both
  (only alphas exist yet).

## The control contract

What `bus-instances`' `control.rs` (and any later control method) implements:

- **Actions:** `list`, `start`, `stop`, `reload`, `reload_config`, `token`, `revoke`,
  `rotate_key`, `server.reload`, `server.update` (`acl::CONTROL_ACTIONS`).
- **Rights:** `control = ["reload:t-1"]` (that action on that instance), `"reload:t-*"`
  (instances whose name starts with `t-`), `"*:t-1"` (every action on `t-1`), `"reload"`
  (that action on every instance), `"*"` (everything). An instance part of `*` is every
  instance, so `"reload:*"` is `"reload"` and `"list:*"` is `"list"` (it reaches
  `bus.instances`), and `"*:*"` is `"*"`: the one reading `bus.token` and `bus.toml` give
  alike (`bus.toml` rights are not checked by `bus.token`'s rule).
- **The call** passes the broker when the token holds the action on some instance
  (`control_gate`); that does not allow acting on any instance.
- **Each instance** the method acts on is checked before it acts, one by one, with
  `authorizer.authorize(&ctx.principal, ctx.call_credential.as_deref(), &AuthzRequest::Control
  { action, instance: Some(name) })`; `name` is the instance's bare name (`t-1`, not
  `instance:t-1`). A call that names no instance ("all") checks every instance it would act on,
  not `instance: None`. One denial: the call is answered `denied` and acts on no instance.
- **The credential:** the call's credential is always passed. Passing `None` instead lets a
  narrow per-call credential on a wide connection act with the connection's rights.

## Open

- `server.proto`'s header (schemas are frozen in this change) is stale: it says `server.reload`
  and `server.update` need control "reload"; they need `server.reload` and `server.update`
  (`server.command`: "owner on the Unix socket only" is right). The change that next edits
  the schema corrects it.
- `bus-core-services`' `server.command` handler checks owner and `Transport::UnixSocket`
  itself as well (see "Deviations"); `server.update`'s handler needs no check of its own.
- A call or stream authorized by a per-call credential runs to its end after that credential
  is revoked (only new calls are refused; a connection that said `Hello` with a revoked token
  is closed). Cancelling calls in flight needs the broker to track which credential each one
  carries, a change of `broker.rs` (not this change's file).
- The shared design's R7 names a 1 ms run limit; this change bounds decisions by the fact and
  iteration limits with a 50 ms backstop (see "Backpressure"); the shared design is updated
  when the change is archived.
