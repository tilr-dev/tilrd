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
| `bus/broker.rs` | bus-broker-core, then bus-streams-events, then bus-reload | routing table, connections, pending calls |
| `bus/transport.rs` | bus-broker-core, then bus-streams-events, then bus-reload | WebSocket transport, Hello |
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

## Deviations in the implementation

What the code froze where it differs from the contract above; the code is the contract.

- **`bus` is a child of `supervisor`.** `supervisor.rs` declares `#[path = "bus/mod.rs"] mod
  bus;` and `main.rs` re-exports it (`use supervisor::bus;`). The files stay in
  `crates/tilrd/src/bus/`, but every `bus::*` module sees the supervisor's private fields and
  helpers without opening `supervisor.rs`. Unit tests are named `supervisor::bus::…` by the test
  runner (and so in USE_CASES.md; `scripts/check_use_cases.py` follows `#[path]` modules).
- **Rules for the lanes** (doc comments of `bus/mod.rs` and `bus/core/mod.rs`): a lane reaches
  the supervisor with free functions in its own file, `fn x(s: &mut Supervisor, …)`, not
  `impl Supervisor` blocks (methods of parallel lanes would collide); a `match` on `CoreCmd`,
  `PaneEvent` or `AuthzRequest` outside its owner ends with `_ => …`.
- **Owners of `broker.rs` and `transport.rs`.** After this change only `bus-streams-events`, then
  `bus-reload`, edit them, one after the other. Every other lane works through the public API
  below; `bus-web` puts its Origin rules in the authorizer (`HelloContext::origin`, `acl.rs`),
  not in `transport.rs`.
- **`Authorizer`.** `hello(&self, hello: &Hello, ctx: &HelloContext<'_>)`, where `HelloContext`
  holds `transport`, `peer`, `origin`, `connection_id` and `seat: Option<&Seat>`: the principal
  carries the connection id, and an instance's identity and rights come from its seat. Added:
  `credential_update(&self, principal, credential: &[u8]) -> Result<Principal,
  ConnectionError>` with a default that refuses (`denied`); the broker calls it for every
  `CredentialUpdate` and, on a refusal, sends the `ConnectionError` and closes the connection.
- **Audit lines.** The broker logs on the target `tilrd::bus::audit` every refused Hello, denied
  call and subscription, refused credential update, and every event it drops (bad or reserved
  subject, denied emit), with connection and identity, so `bus-rights` needs no edit for them.
- **`attach_stream(fd: OwnedFd, seat: Seat) -> Attached`** instead of a `Principal`: the
  principal exists only after Hello. `Seat { identity, rights, transport, peer, check }` is what
  the runner knows before (`check` verifies the Hello, e.g. the instance token); `Attached {
  connection_id, state: watch::Receiver<ConnState> }` tells the runner when the participant is
  greeted (`Up`) and gone (`Ended(Ended { reason, refusal: Option<ConnectionError> })`, the first
  end kept: a refusal's code, e.g. `reserved`, reaches the runner).
- **More of `BrokerHandle`** for the lanes: `ConnInfo::provides` is `Vec<MethodSpec>` (kind, lane,
  select); `drain_connection(id, timeout) -> bool` (no new calls by method, wait for the calls
  sent to it, then `unavailable` and `Cancel`; the connection stays open); `resume_routing()`
  undoes `stop_routing`; `update_principal(conn, Principal)` and `principal_of(conn)`;
  `set_draining`, `close_connection`.
- **`CancelToken`** (own: `cancel`, `is_cancelled`, `async cancelled`) instead of
  `tokio_util::CancellationToken`: no new dependency.
- **A unary native handler returns `Result<Reply, BusError>`** (`Reply { payload,
  content_type }` of `tilr_bus`), not `Bytes`: JSON callers get `application/json` back.
  `NativeMethod::unary(name, f)`, `NativeMethod::stream(name, lane, f)` build one.
- **`StreamOut::caught_up` is an `async fn`**, not a `Notified`.
- **A native call has a generation.** A caller may reuse an id once it cancelled it; the late end
  of the old handler (`StreamOut` dropped, a unary result) is matched by generation and does not
  touch the new call.
- **The end of a stream goes behind its chunks**: `End` or the error `Reply` of a bulk stream
  is queued in the bulk lane, native or relayed.
- **`BrokerHandle::call(from, method, payload, CallOptions)`**: the target is
  `CallOptions::target`, as in the client. The broker's pending calls live in `broker.rs`.
- **Supervisor side.** The supervisor holds `bus: bus::core::CoreState` (the observers, the
  broker handle and typed slots: `CoreState::slot::<T>()`, read-only `slot_ref::<T>()`). The
  hooks are placed: `core::pane_output(&Supervisor, …)` in `on_pane_output`, `core::pane_event(
  &Supervisor, &PaneEvent::{Created, Changed, Activity, Exited})` in `on_pane_started`, the
  `PaneStatus` arm, before `broadcast_activity`, and in `on_pane_link_closed`, each only while
  an observer listens (`CoreState::observing`). `BusObserver::pane_output(&self, &Supervisor,
  &PaneId, FrameHead, &Bytes)` and `pane_event(&self, &Supervisor, &PaneEvent)` read the
  supervisor synchronously and have empty defaults. The registry takes observers
  (`Registry::observe`). `core::Core { supervisor, broker, features }` is what a `register` gets;
  `Features` mirrors the `tilr.v1` feature switches (B14).
- **`bus.ping`** lives in `broker.rs` (`broker::ping()`), registered first by `core::registry`.
- **Greeting waits for the core.** A Hello is answered only once `BrokerHandle::install` ran
  (`BrokerHandle::ready`); before it nothing would route to `tilrd`.
- **An invalid method name in `provides`** is `ConnectionError{bad_request}`; a reserved one
  `reserved`, `"<method> is in the namespace <ns> of tilrd"`. A first message that is not an
  envelope is `expected_hello` and closes the connection, as anything else first does.
- **The bus WebSocket gets its own limits.** The handshake runs with the `tilr.v1` config;
  after it the stream is re-wrapped (`WebSocketStream::from_raw_socket`) with a 17 MiB message
  and frame limit (16 MiB + slack). No bytes are lost: a client sends nothing before the
  101 response (RFC 6455 §4.1).
- **Connection ids** come from the same counter as `tilr.v1` client ids, which survives a
  reload, so an id is never reused within one server's life.
- **Test harness.** `BusServer::start` runs `--listen 127.0.0.1:0` (the `tilr.v1` harness's
  `Server::start_full`), not `--no-listen`, and derefs to `Server`. It adds `raw()` /
  `raw_greeted()` (envelopes by hand) and `greeted(name)` / `wait_greeted` (by the log line
  `participant greeted`). It builds the test runtime itself, once per test process (`cargo build
  -p tilr-bus --example test_runtime --locked`, same profile and target directory), so a test
  does not depend on a prebuilt example.
- **Emits from participants** to a reserved or invalid subject are dropped (no reply exists for
  an `Event`), with an audit line.
