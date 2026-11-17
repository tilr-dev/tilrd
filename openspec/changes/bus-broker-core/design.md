# Design: The bus broker in tilrd beside tilr.v1

Shared design: `openspec/design/unified-bus.md` (B1-B8, B12, B14, O2, O4). This file holds this
change's part and the **frozen contract** the other changes code against. Items marked
*(added by the split)* are not in the shared design; they are the minimum the lanes need and are
open to a small change in this change's PR, frozen once it merges.

## Decisions

- **Skeleton first.** The broker routes calls, plain streams and events. Credits are accepted
  and relayed but not enforced (every stream has unlimited credit), queues are the existing
  `outbox.rs` lanes with their current bounds, and there are no deadlines; `bus-streams-events`
  adds those behind the same types.
- **Tests without instances.** Instances are `bus-instances`'s. The tests of this change run the
  test runtime of `bus-wire` as WebSocket participants on the Unix socket (identity `owner`,
  told apart by `client_name`, which `test.whoami` answers before `bus-instances`); scenarios
  that name instances `t-N` are run that way here, and their instance-only clauses (a restart
  after `reserved`) are checked by `bus-instances`.
- **Stub authorizer** *(added by the split)*: until `bus-rights`, the authorizer allows `tilrd`
  and `owner` everything, allows an instance what its `Rights` (the `rights` table of R3) match
  as plain patterns (exact, `ns.*` prefix, `*` without `bus.*`; `control` as `action[:instance]`),
  allows every pane request (T7 is `bus-pane-rights`'s), and refuses every credential at Hello
  (`ConnectionError{denied}`, `tokens are not supported by this build`): fail closed.

## Frozen contract

### Module tree
`crates/tilrd/src/bus/mod.rs` declares every submodule below; each is created here (a stub
holds only a comment naming its owner). Only the owner edits a file; a file shared in time is
edited by the later change only after the earlier one merged.

| file | owner | what |
|---|---|---|
| `bus/broker.rs` | bus-broker-core | routing table, connections, pending calls |
| `bus/transport.rs` | bus-broker-core (`bus-web`: Origin) | WebSocket transport, Hello |
| `bus/identity.rs` | bus-broker-core | `Identity`, `Rights`, `Principal`, `CallCtx` |
| `bus/authz.rs` | bus-broker-core (`bus-rights`: `default_authorizer` body) | `Authorizer`, `AuthzRequest`, stub |
| `bus/events.rs`, `bus/streams.rs` | bus-broker-core, then bus-streams-events | events; streams, credits, lanes |
| `bus/runner.rs`, `bus/config.rs`, `bus/control.rs` | bus-instances | spawn, `bus.toml`, `bus.*` control |
| `bus/acl.rs` | bus-rights (`bus-web`: origin rules) | Biscuit |
| `bus/state.rs` | bus-reload | `bus-state 1` |
| `bus/core/mod.rs` | bus-broker-core | the native participant `tilrd`, method registry |
| `bus/core/json.rs`, `bus/core/panes.rs` | bus-core-panes | JSON mapping; `pane.*` |
| `bus/core/profiles.rs` | pane-profiles | `pane.profiles`, `pane.create {profile}` resolution |
| `bus/core/pane_rights.rs` | bus-pane-rights | T7 |
| `bus/core/{server,fs,exec,clipboard,host,agent}.rs` | bus-core-services | T2 rows of these namespaces |

`bus/core/mod.rs` calls one `register(&mut Registry)` per core file (stubs register nothing),
so a lane adds methods without editing `core/mod.rs`.

### Types *(added by the split, from B2, B4, B5, R3, R6, T7)*
- `enum Identity { Tilrd, Owner, Instance(String), Token(String) }`, displayed as `tilrd`,
  `owner`, `instance:<name>`, `token:<name>` (B2).
- `struct Rights { call, provide, emit, subscribe, control, pane: Vec<String> }`: the strings of
  R3 as written in `[instances.<n>.rights]` (serde), parsed by `bus-instances`, minted from by
  `bus-rights`.
- `struct Principal { identity: Identity, rights: Option<Arc<Rights>>, credential:
  Option<Arc<dyn Any + Send + Sync>> /* the verified token, None before bus-rights */, origin:
  String /* "" without */, transport: Transport, client_id: String, client_name: String,
  connection_id: u64 }`; `enum Transport { UnixSocket, Tcp, InstanceFd, InstanceSocket }`.
- `struct CallCtx { id: u64, method: String, target: String, from: Identity, principal:
  Arc<Principal>, call_credential: Option<Bytes>, deadline: Option<Instant>, cancel:
  CancellationToken, content_type: String, trace_id: String }`.

### Traits *(added by the split)*
- `trait Authorizer: Send + Sync { fn hello(&self, hello: &Hello, transport: Transport, peer:
  Peer, origin: &str) -> Result<Principal, ConnectionError>; fn authorize(&self, p: &Principal,
  call_credential: Option<&[u8]>, req: &AuthzRequest<'_>) -> Result<(), Denied>; fn
  hello_ack_credential(&self, p: &Principal) -> Option<Bytes>; }` with `enum AuthzRequest<'a> {
  Call{method, target}, Provide{method}, Emit{subject}, Subscribe{pattern}, Control{action,
  instance}, Event{subject, scope}, PaneCreate{profile, overrides}, PaneAccess{pane_id,
  creator} }` and `struct Denied { reason: String }` (the `denied` message, naming the missing
  right). `pub fn default_authorizer(cfg: &AuthzConfig) -> Arc<dyn Authorizer>` returns the stub;
  `bus-rights` replaces its body, nothing else.
- Native participant: `struct NativeMethod { spec: MethodSpec, handler: NativeHandler }`, `enum
  NativeHandler { Unary(Arc<dyn Fn(CallCtx, Bytes) -> BoxFuture<'static, Result<Bytes,
  BusError>> + Send + Sync>), Stream(Arc<dyn Fn(CallCtx, Bytes, StreamOut) -> BoxFuture<'static,
  ()> + Send + Sync>) }`; `Registry::add(NativeMethod)`; a feature turned off registers nothing
  (B14).
- `StreamOut` (one stream, provider side): `async fn send(&self, Chunk) -> Result<(), Closed>`
  (waits for credit once `bus-streams-events` enforces it), `fn try_send(&self, Chunk) ->
  Result<(), TrySendError>` (`Full` when it would wait), `fn is_lagging(&self) -> bool` (the
  consumer's interactive lane held a packet over 250 ms; `bus-streams-events` adds "no credit"),
  `fn caught_up(&self) -> Notified` (T4), `fn cancelled(&self) -> &CancellationToken`, `fn
  end(self, payload: Bytes)`, `fn fail(self, BusError)`.
- `BrokerHandle` (clone, `Send`): `emit(Identity::Tilrd, subject, payload, scope: Option<String>)`;
  `call(from: Identity, target: &str, method, payload, CallOptions) -> BoxFuture<Result<Reply,
  BusError>>` (tilrd calls `host.open`, `ssh_agent.*`); `attach_stream(fd: OwnedFd, principal:
  Principal)` (used by `bus-instances`); `connections() -> Vec<ConnInfo>`; `providers(method) ->
  Vec<ConnInfo>`; `stop_routing(reason)` and `drain(timeout)` (used by `bus-reload`).
- `SupervisorHandle` (clone): an `mpsc` sender of `enum CoreCmd`, received in the supervisor's
  loop. `CoreCmd::With(Box<dyn FnOnce(&mut Supervisor) + Send>)` runs a closure written in
  `bus/core/*.rs` on the supervisor's own task (the existing paths for create, input with
  acknowledgement, resize, close, frames, files, exec, clipboard); a lane that needs a typed
  request appends a variant at the end of `CoreCmd` with its match arm. Supervisor events reach
  the bus through `trait BusObserver { fn pane_output(&self, pane: &PaneId, head: FrameHead,
  bytes: Bytes); fn pane_event(&self, ev: PaneEvent); }`, a list held by the supervisor; a lane
  adds the one-line call site it needs (a hook, named in its design) and implements the trait in
  its own file.

### Test harness
`crates/tilrd/tests/common/bus.rs`: `BusServer::start(BusOptions)` (tilrd with temporary
config, state and runtime directories, `--no-listen` plus a test port, optional `bus.toml` and
`server.toml` text, failpoints), `owner()` (a `tilr_bus::Bus` on the Unix socket), `tcp(access
token | credential)`, `test_runtime(client_name, env)` (the test runtime as a WebSocket
participant), `log_lines()`, `reload()`, `kill()`. Stubs `tests/common/bus_{streams,instances,
rights,panes,services,profiles,sandbox,web,reload}.rs` for the lanes' helpers. `tests/common/
mod.rs` gets only the `pub mod bus…;` lines (O4).

## Reload and reconnect

A reload ends every bus WebSocket connection, as it ends `tilr.v1` clients; participants
reconnect and get a new `connection_id`. Nothing of the broker crosses the exec in this change
(instances come with `bus-instances`, their handover with `bus-reload`).

## Backpressure

Each bus connection uses the existing two-lane outbox (`outbox.rs`): 1024 interactive envelopes
(full → 4001), bulk 4 MiB. Credits, the per-connection window and `busy` come with
`bus-streams-events` (B7, B8).
