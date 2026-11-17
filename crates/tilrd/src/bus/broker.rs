//! The broker: connections, the routing table, pending calls and streams (design
//! B1, B2, B4-B7, B11, B12). Owner: `bus-broker-core`.
//!
//! - A `Call` with a `target` goes to that participant; one without goes to a live,
//!   non-draining provider of the method by its `Select` (in turn in connection
//!   order, or the provider that sent `Activate` last, else the last connected).
//!   The namespaces of [`tilr_bus::names::RESERVED_NAMESPACES`] are the native
//!   participant `tilrd`'s.
//! - `from` is the sender's identity, whatever it wrote. Call ids are remapped per
//!   hop, so a `Reply`, `Chunk` or `End` is taken only from the connection the call
//!   went to; a late or forged one is dropped.
//! - Envelopes of one connection are handled in the order read and queued in the
//!   callee's lanes in that order: calls (and cancels) between two connections keep
//!   their order. The native participant's handler functions are called in that
//!   order too ([`super::NativeHandler`]).
//! - A stream is a call to a `STREAM` method: `Chunk*`, then `End` or an error
//!   `Reply`; a `Reply` with a payload becomes `bad_reply`. The caller's `Cancel` is
//!   forwarded and later chunks are dropped. `Credit` is relayed, not enforced
//!   (`bus-streams-events`).
//! - A connection's end answers the calls sent to it `unavailable` and cancels the
//!   calls it made.
//!
//! The state is one mutex, held for bookkeeping only: never across an await, an
//! authorization or an encoding (B8's split is `bus-streams-events`').

use std::collections::{BTreeMap, HashMap};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use tilr_bus::bus::{
    Call, Cancel, ConnectionError, Credit, End, Envelope, Error as WireError, Event as WireEvent,
    Kind, Lane, MethodSpec, Reply as WireReply, Select, Subscribe, Unsubscribe, envelope::Msg,
    reply,
};
use tilr_bus::{BusError, CallOptions, Reply, codes, names};
use tokio::sync::{oneshot, watch};

use super::authz::{Authorizer, AuthzRequest};
use super::events::Subscriptions;
use super::identity::{CallCtx, CancelToken, Identity, Principal, Seat, Transport};
use super::streams::{Chunk, Closed, NativeHandler, NativeMethod, StreamOut, TrySendError};
use super::transport::Out;

/// The broker's id of a connection, `HelloAck.connection_id`: from the counter the
/// `tilr.v1` connections take theirs from, so it grows across reloads.
pub type ConnId = u64;

/// What the broker is made with.
pub struct BrokerOptions {
    pub authz: Arc<dyn Authorizer>,
    pub server_id: String,
    /// Shared with the `tilr.v1` connections; handed on in a reload.
    pub next_connection_id: Arc<AtomicU64>,
    /// Drop a WebSocket participant nothing was heard from for this long.
    pub client_timeout: Option<Duration>,
}

/// The broker, as everything else holds it. Clones share it.
#[derive(Clone)]
pub struct BrokerHandle(Arc<Shared>);

struct Shared {
    state: Mutex<State>,
    authz: Arc<dyn Authorizer>,
    server_id: String,
    next_connection_id: Arc<AtomicU64>,
    next_hop: AtomicU64,
    client_timeout: Option<Duration>,
    /// The native participant's methods, by name; set once the core is built.
    native: OnceLock<HashMap<String, NativeMethod>>,
    /// Set once `native` is.
    ready: watch::Sender<bool>,
}

#[derive(Default)]
struct State {
    /// By id, so in connection order.
    conns: BTreeMap<ConnId, Conn>,
    /// The providers of each method, in connection order.
    providers: HashMap<String, Vec<ConnId>>,
    /// Round-robin turns, per method.
    turns: HashMap<String, usize>,
    /// Counts `Activate`s, to tell which came last.
    activations: u64,
    /// Set by [`BrokerHandle::stop_routing`]: new calls are answered `unavailable`.
    stopped: Option<String>,
}

struct Conn {
    principal: Arc<Principal>,
    /// The identity's text, as `from` and `target` name it.
    identity: String,
    out: Out,
    provides: HashMap<String, MethodSpec>,
    /// [`State::activations`] at this connection's last `Activate`; 0: never.
    activated: u64,
    draining: bool,
    subscriptions: Subscriptions,
    /// Calls and streams this connection made, by its id.
    calls: HashMap<u64, Route>,
    /// Calls and streams sent to this connection, by hop id.
    served: HashMap<u64, Served>,
    state: watch::Sender<ConnState>,
}

/// Where a pending call went.
enum Route {
    Remote {
        callee: ConnId,
        hop: u64,
    },
    Native {
        /// Tells this call from a later one that reuses its id.
        generation: u64,
        cancel: CancelToken,
        abort: Option<tokio::task::AbortHandle>,
    },
}

impl Route {
    /// Stop a native call's handler (outside the lock).
    fn stop(self) {
        if let Route::Native { cancel, abort, .. } = self {
            cancel.cancel();
            if let Some(abort) = abort {
                abort.abort();
            }
        }
    }
}

/// A call a connection is to answer.
struct Served {
    caller: Caller,
    kind: Kind,
    lane: Lane,
}

enum Caller {
    Conn {
        conn: ConnId,
        id: u64,
    },
    /// tilrd's core, waiting in [`BrokerHandle::call`].
    Native(oneshot::Sender<Result<Reply, BusError>>),
}

/// How an attached connection is doing, for its runner.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnState {
    /// Waiting for `Hello`.
    AwaitingHello,
    /// Greeted: `HelloAck` sent.
    Up,
    /// The connection ended; the first end set stays.
    Ended(Ended),
}

/// Why a connection ended.
#[derive(Clone, Debug, PartialEq)]
pub struct Ended {
    /// In words: `disconnected`, `the instance closed its end`, `bad frame: …`.
    pub reason: String,
    /// The `ConnectionError` it was turned away with (Hello refused), if it was.
    pub refusal: Option<ConnectionError>,
}

impl ConnState {
    /// Set `Ended` unless it ended already (the first reason is kept).
    pub(super) fn end(
        state: &watch::Sender<ConnState>,
        reason: impl Into<String>,
        refusal: Option<ConnectionError>,
    ) {
        let ended = Ended {
            reason: reason.into(),
            refusal,
        };
        state.send_if_modified(|current| {
            if matches!(current, ConnState::Ended(_)) {
                return false;
            }
            *current = ConnState::Ended(ended);
            true
        });
    }
}

/// What [`BrokerHandle::attach_stream`] gives the runner of an instance.
pub struct Attached {
    pub connection_id: ConnId,
    pub state: watch::Receiver<ConnState>,
}

/// A connection, as `bus.instances` and the control methods list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnInfo {
    pub connection_id: ConnId,
    pub identity: String,
    pub client_name: String,
    pub client_id: String,
    pub transport: Transport,
    /// What it provides, with kind, lane and select, sorted by method.
    pub provides: Vec<MethodSpec>,
    pub draining: bool,
}

enum Callee {
    Native(NativeMethod),
    Conn(ConnId, MethodSpec),
}

fn error_reply(id: u64, error: BusError) -> Msg {
    Msg::Reply(WireReply {
        id,
        result: Some(reply::Result::Error(WireError {
            code: error.code,
            message: error.message,
        })),
        content_type: String::new(),
    })
}

fn payload_reply(id: u64, reply: Reply) -> Msg {
    Msg::Reply(WireReply {
        id,
        result: Some(reply::Result::Payload(reply.payload)),
        content_type: reply.content_type,
    })
}

fn wire_to_result(reply: WireReply) -> Result<Reply, BusError> {
    match reply.result {
        Some(reply::Result::Error(error)) => Err(BusError::new(error.code, error.message)),
        Some(reply::Result::Payload(payload)) => Ok(Reply {
            payload,
            content_type: reply.content_type,
        }),
        None => Ok(Reply {
            payload: Bytes::new(),
            content_type: reply.content_type,
        }),
    }
}

fn kind_of(spec: &MethodSpec) -> Kind {
    Kind::try_from(spec.kind).unwrap_or(Kind::Unary)
}

fn lane_of(spec: &MethodSpec) -> Lane {
    Lane::try_from(spec.lane).unwrap_or(Lane::Interactive)
}

impl BrokerHandle {
    pub fn new(options: BrokerOptions) -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State::default()),
            authz: options.authz,
            server_id: options.server_id,
            next_connection_id: options.next_connection_id,
            next_hop: AtomicU64::new(1),
            client_timeout: options.client_timeout,
            native: OnceLock::new(),
            ready: watch::channel(false).0,
        }))
    }

    /// Install the native participant's methods; once. Connections are greeted
    /// only after it ([`ready`](Self::ready)).
    pub fn install(&self, methods: Vec<NativeMethod>) {
        let methods = methods
            .into_iter()
            .map(|method| (method.spec.method.clone(), method))
            .collect();
        if self.0.native.set(methods).is_err() {
            tracing::error!(target: "tilrd::bus", "the bus core was installed twice");
        }
        self.0.ready.send_replace(true);
    }

    /// Returns once the core is installed: the server is up.
    pub async fn ready(&self) {
        let mut ready = self.0.ready.subscribe();
        let _ = ready.wait_for(|ready| *ready).await;
    }

    pub fn server_id(&self) -> &str {
        &self.0.server_id
    }

    pub fn authorizer(&self) -> &Arc<dyn Authorizer> {
        &self.0.authz
    }

    pub fn client_timeout(&self) -> Option<Duration> {
        self.0.client_timeout
    }

    /// A new connection's id.
    pub fn next_connection_id(&self) -> ConnId {
        self.0.next_connection_id.fetch_add(1, Ordering::Relaxed)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn hop(&self) -> u64 {
        self.0.next_hop.fetch_add(1, Ordering::Relaxed)
    }

    /// The native participant's method `name`.
    fn native_method(&self, name: &str) -> Result<NativeMethod, BusError> {
        let Some(methods) = self.0.native.get() else {
            return Err(BusError::unavailable("tilrd is starting"));
        };
        methods
            .get(name)
            .cloned()
            .ok_or_else(|| BusError::new(codes::NOT_FOUND, format!("no method {name}")))
    }

    /// The methods `tilrd` provides, sorted.
    pub fn native_methods(&self) -> Vec<MethodSpec> {
        let mut specs: Vec<MethodSpec> = self
            .0
            .native
            .get()
            .map(|methods| methods.values().map(|m| m.spec.clone()).collect())
            .unwrap_or_default();
        specs.sort_by(|a, b| a.method.cmp(&b.method));
        specs
    }

    // -----------------------------------------------------------------------
    // Connections
    // -----------------------------------------------------------------------

    /// Take a greeted connection in: its `HelloAck` is queued already, so nothing
    /// reaches it before. A connection of the same principal and `client_id`
    /// still open is closed 4000 (B12).
    pub(super) fn register(
        &self,
        principal: Principal,
        provides: Vec<MethodSpec>,
        out: Out,
        state: watch::Sender<ConnState>,
    ) {
        let id = principal.connection_id;
        let identity = principal.identity.to_string();
        let superseded: Vec<ConnId> = {
            let st = self.lock();
            st.conns
                .iter()
                .filter(|(_, conn)| {
                    !principal.client_id.is_empty()
                        && conn.identity == identity
                        && conn.principal.client_id == principal.client_id
                })
                .map(|(id, _)| *id)
                .collect()
        };
        for old in superseded {
            tracing::info!(target: "tilrd::bus", connection = old, by = id, "superseded");
            if let Some(out) = self.disconnect(old) {
                out.close(
                    tilr_bus::close::SUPERSEDED,
                    tilr_bus::close::SUPERSEDED_REASON,
                );
            }
        }
        let provides: HashMap<String, MethodSpec> = provides
            .into_iter()
            .map(|spec| (spec.method.clone(), spec))
            .collect();
        state.send_replace(ConnState::Up);
        let mut st = self.lock();
        for method in provides.keys() {
            st.providers.entry(method.clone()).or_default().push(id);
        }
        st.conns.insert(
            id,
            Conn {
                principal: Arc::new(principal),
                identity,
                out,
                provides,
                activated: 0,
                draining: false,
                subscriptions: Subscriptions::default(),
                calls: HashMap::new(),
                served: HashMap::new(),
                state,
            },
        );
    }

    /// Forget a connection: the calls sent to it are answered `unavailable`, the
    /// calls it made are cancelled. Returns its writer, for the caller to close.
    pub(super) fn disconnect(&self, id: ConnId) -> Option<Out> {
        let mut sends: Vec<(Out, Msg)> = Vec::new();
        let mut natives: Vec<oneshot::Sender<Result<Reply, BusError>>> = Vec::new();
        let mut conn = {
            let mut st = self.lock();
            let conn = st.conns.remove(&id)?;
            for method in conn.provides.keys() {
                if let Some(list) = st.providers.get_mut(method) {
                    list.retain(|provider| *provider != id);
                    if list.is_empty() {
                        st.providers.remove(method);
                        st.turns.remove(method);
                    }
                }
            }
            let gone = format!("{} disconnected", conn.identity);
            for served in conn.served.values() {
                match &served.caller {
                    Caller::Conn { conn: caller, id } => {
                        if let Some(caller) = st.conns.get_mut(caller) {
                            caller.calls.remove(id);
                            sends.push((
                                caller.out.clone(),
                                error_reply(*id, BusError::unavailable(gone.clone())),
                            ));
                        }
                    }
                    Caller::Native(_) => {}
                }
            }
            for route in conn.calls.values() {
                if let Route::Remote { callee, hop } = route
                    && let Some(callee) = st.conns.get_mut(callee)
                {
                    callee.served.remove(hop);
                    sends.push((
                        callee.out.clone(),
                        Msg::Cancel(Cancel {
                            id: *hop,
                            reason: "the caller's connection ended".into(),
                        }),
                    ));
                }
            }
            conn
        };
        for (_, route) in conn.calls.drain() {
            route.stop();
        }
        let Conn {
            out,
            served,
            state,
            identity,
            ..
        } = conn;
        for served in served.into_values() {
            if let Caller::Native(tx) = served.caller {
                natives.push(tx);
            }
        }
        for tx in natives {
            let _ = tx.send(Err(BusError::unavailable(format!(
                "{identity} disconnected"
            ))));
        }
        for (out, msg) in sends {
            out.send(msg, Lane::Interactive);
        }
        ConnState::end(&state, "disconnected", None);
        Some(out)
    }

    /// Close a connection with a WebSocket close code (on a stream socket: close it).
    pub fn close_connection(&self, id: ConnId, code: u16, reason: &'static str) {
        let out = self.lock().conns.get(&id).map(|conn| conn.out.clone());
        if let Some(out) = out {
            out.close(code, reason);
        }
    }

    /// A draining connection gets no new calls routed by method (B5).
    pub fn set_draining(&self, id: ConnId, draining: bool) {
        if let Some(conn) = self.lock().conns.get_mut(&id) {
            conn.draining = draining;
        }
    }

    /// The connections open now, in connection order.
    pub fn connections(&self) -> Vec<ConnInfo> {
        self.lock().conns.values().map(conn_info).collect()
    }

    /// The live connections that provide `method`, in connection order.
    pub fn providers(&self, method: &str) -> Vec<ConnInfo> {
        let st = self.lock();
        st.providers
            .get(method)
            .into_iter()
            .flatten()
            .filter_map(|id| st.conns.get(id))
            .map(conn_info)
            .collect()
    }

    /// Stop routing new calls (B13): they are answered `unavailable` with `reason`.
    pub fn stop_routing(&self, reason: &str) {
        self.lock().stopped = Some(reason.to_owned());
    }

    /// Route new calls again after [`stop_routing`](Self::stop_routing) (a reload
    /// that failed and carries on).
    pub fn resume_routing(&self) {
        self.lock().stopped = None;
    }

    /// Drain one connection (an instance being stopped or replaced): it gets no new
    /// calls routed by method; wait up to `timeout` for the calls sent to it, then
    /// answer the rest `unavailable` and cancel them at it. Returns whether every
    /// call sent to it was answered in time. The connection stays open.
    pub async fn drain_connection(&self, id: ConnId, timeout: Duration) -> bool {
        self.set_draining(id, true);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let pending = self
                .lock()
                .conns
                .get(&id)
                .is_some_and(|conn| !conn.served.is_empty());
            if !pending {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let reason = "the provider is stopping";
        let mut sends = Vec::new();
        let mut natives = Vec::new();
        {
            let mut st = self.lock();
            let Some(conn) = st.conns.get_mut(&id) else {
                return false;
            };
            let callee_out = conn.out.clone();
            let served: Vec<(u64, Served)> = conn.served.drain().collect();
            for (hop, served) in served {
                sends.push((
                    callee_out.clone(),
                    Msg::Cancel(Cancel {
                        id: hop,
                        reason: reason.into(),
                    }),
                ));
                match served.caller {
                    Caller::Conn { conn: caller, id } => {
                        if let Some(caller) = st.conns.get_mut(&caller) {
                            caller.calls.remove(&id);
                            sends.push((
                                caller.out.clone(),
                                error_reply(id, BusError::unavailable(reason)),
                            ));
                        }
                    }
                    Caller::Native(tx) => natives.push(tx),
                }
            }
        }
        for tx in natives {
            let _ = tx.send(Err(BusError::unavailable(reason)));
        }
        for (out, msg) in sends {
            out.send(msg, Lane::Interactive);
        }
        false
    }

    /// Wait up to `timeout` for the calls in flight, then answer the rest
    /// `unavailable` and cancel them at their callees.
    pub async fn drain(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let pending = self
                .lock()
                .conns
                .values()
                .any(|conn| !conn.calls.is_empty() || !conn.served.is_empty());
            if !pending || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let reason = self
            .lock()
            .stopped
            .clone()
            .unwrap_or_else(|| "tilrd is reloading".into());
        let mut sends = Vec::new();
        let mut natives = Vec::new();
        let mut stopped = Vec::new();
        {
            let mut st = self.lock();
            let ids: Vec<ConnId> = st.conns.keys().copied().collect();
            for id in ids {
                let Some(conn) = st.conns.get_mut(&id) else {
                    continue;
                };
                let calls: Vec<(u64, Route)> = conn.calls.drain().collect();
                let caller_out = conn.out.clone();
                for (call_id, route) in calls {
                    sends.push((
                        caller_out.clone(),
                        error_reply(call_id, BusError::unavailable(reason.clone())),
                    ));
                    match route {
                        Route::Remote { callee, hop } => {
                            if let Some(callee) = st.conns.get_mut(&callee) {
                                callee.served.remove(&hop);
                                sends.push((
                                    callee.out.clone(),
                                    Msg::Cancel(Cancel {
                                        id: hop,
                                        reason: reason.clone(),
                                    }),
                                ));
                            }
                        }
                        native => stopped.push(native),
                    }
                }
            }
            for conn in st.conns.values_mut() {
                for (_, served) in conn.served.drain() {
                    if let Caller::Native(tx) = served.caller {
                        natives.push(tx);
                    }
                }
            }
        }
        for route in stopped {
            route.stop();
        }
        for tx in natives {
            let _ = tx.send(Err(BusError::unavailable(reason.clone())));
        }
        for (out, msg) in sends {
            out.send(msg, Lane::Interactive);
        }
    }

    /// Attach an instance's stream socket (descriptor 3 or an accepted instance
    /// socket): the broker reads its `Hello` and serves it as the seat says.
    pub fn attach_stream(&self, fd: OwnedFd, seat: Seat) -> Attached {
        let connection_id = self.next_connection_id();
        let (state, watched) = watch::channel(ConnState::AwaitingHello);
        super::transport::spawn_stream(self.clone(), connection_id, fd, seat, state);
        Attached {
            connection_id,
            state: watched,
        }
    }

    // -----------------------------------------------------------------------
    // Envelopes from a connection
    // -----------------------------------------------------------------------

    /// Handle one envelope a greeted connection sent.
    pub(super) fn on_envelope(&self, from: ConnId, envelope: Envelope) {
        let Some(msg) = envelope.msg else {
            return; // A variant of a later version.
        };
        match msg {
            Msg::Call(call) => self.on_call(from, call),
            Msg::Reply(reply) => self.on_reply(from, reply),
            Msg::Chunk(chunk) => self.on_chunk(from, chunk),
            Msg::End(end) => self.on_end(from, end),
            Msg::Cancel(cancel) => self.on_cancel(from, cancel),
            Msg::Credit(credit) => self.on_credit(from, credit),
            Msg::Event(event) => self.on_event(from, event),
            Msg::Subscribe(subscribe) => self.on_subscribe(from, subscribe),
            Msg::Unsubscribe(Unsubscribe { id }) => {
                if let Some(conn) = self.lock().conns.get_mut(&from) {
                    conn.subscriptions.remove(id);
                }
            }
            Msg::Activate(_) => {
                let mut st = self.lock();
                st.activations += 1;
                let turn = st.activations;
                if let Some(conn) = st.conns.get_mut(&from) {
                    conn.activated = turn;
                }
            }
            Msg::CredentialUpdate(update) => self.on_credential_update(from, &update.credential),
            Msg::Hello(_) | Msg::HelloAck(_) | Msg::ConnectionError(_) => {}
        }
    }

    /// A renewal: the authorizer says who the connection is now, or it is closed.
    fn on_credential_update(&self, from: ConnId, credential: &[u8]) {
        let Some((principal, out)) = self.principal(from) else {
            return;
        };
        match self.0.authz.credential_update(&principal, credential) {
            Ok(renewed) => self.update_principal(from, renewed),
            Err(error) => {
                tracing::info!(target: "tilrd::bus::audit", connection = from, identity = %principal.identity, code = error.code, "credential update refused: {}", error.message);
                out.send(Msg::ConnectionError(error), Lane::Interactive);
                out.flush_close();
            }
        }
    }

    /// Replace a connection's principal (a renewed or narrowed credential); the
    /// calls it makes from now on are authorized as it. Its connection id stays.
    pub fn update_principal(&self, conn: ConnId, principal: Principal) {
        if let Some(conn) = self.lock().conns.get_mut(&conn) {
            conn.identity = principal.identity.to_string();
            conn.principal = Arc::new(Principal {
                connection_id: conn.principal.connection_id,
                ..principal
            });
        }
    }

    /// Who a connection is now (its credential included), while it is open.
    pub fn principal_of(&self, conn: ConnId) -> Option<Arc<Principal>> {
        self.lock()
            .conns
            .get(&conn)
            .map(|conn| Arc::clone(&conn.principal))
    }

    fn principal(&self, conn: ConnId) -> Option<(Arc<Principal>, Out)> {
        self.lock()
            .conns
            .get(&conn)
            .map(|conn| (Arc::clone(&conn.principal), conn.out.clone()))
    }

    fn on_call(&self, from: ConnId, call: Call) {
        let Some((principal, out)) = self.principal(from) else {
            return;
        };
        let id = call.id;
        if !names::valid_method(&call.method) {
            out.send(
                error_reply(
                    id,
                    BusError::bad_request(format!("{:?} is not a method name", call.method)),
                ),
                Lane::Interactive,
            );
            return;
        }
        let credential = (!call.credential.is_empty()).then_some(call.credential.as_ref());
        if let Err(denied) = self.0.authz.authorize(
            &principal,
            credential,
            &AuthzRequest::Call {
                method: &call.method,
                target: &call.target,
            },
        ) {
            tracing::info!(target: "tilrd::bus::audit", connection = from, identity = %principal.identity, method = call.method, "call denied: {denied}");
            out.send(error_reply(id, denied.into()), Lane::Interactive);
            return;
        }
        // Decided under the lock, sent after it.
        let decided = {
            let mut st = self.lock();
            self.route_call(&mut st, from, &principal, call)
        };
        match decided {
            Err(error) => {
                out.send(error_reply(id, error), Lane::Interactive);
            }
            Ok(Routed::Remote(callee_out, forwarded)) => {
                callee_out.send(Msg::Call(forwarded), Lane::Interactive);
            }
            Ok(Routed::Native(method, ctx, payload, generation)) => {
                self.run_native(from, method, ctx, payload, generation);
            }
        }
    }

    fn route_call(
        &self,
        st: &mut State,
        from: ConnId,
        principal: &Arc<Principal>,
        call: Call,
    ) -> Result<Routed, BusError> {
        if let Some(reason) = &st.stopped {
            return Err(BusError::unavailable(reason.clone()));
        }
        let caller = st
            .conns
            .get(&from)
            .ok_or_else(|| BusError::unavailable("gone"))?;
        if caller.calls.contains_key(&call.id) {
            return Err(BusError::bad_request(format!(
                "call id {} is in use on this connection",
                call.id
            )));
        }
        let callee = self.resolve(st, &call.method, &call.target)?;
        let identity = principal.identity.to_string();
        match callee {
            Callee::Native(method) => {
                let cancel = CancelToken::new();
                let generation = self.hop();
                let ctx = CallCtx {
                    id: call.id,
                    method: call.method,
                    target: call.target,
                    from: principal.identity.clone(),
                    principal: Arc::clone(principal),
                    call_credential: (!call.credential.is_empty()).then_some(call.credential),
                    deadline: None,
                    cancel: cancel.clone(),
                    content_type: call.content_type,
                    trace_id: call.trace_id,
                };
                st.conns.get_mut(&from).expect("present").calls.insert(
                    ctx.id,
                    Route::Native {
                        generation,
                        cancel,
                        abort: None,
                    },
                );
                Ok(Routed::Native(method, ctx, call.payload, generation))
            }
            Callee::Conn(callee, spec) => {
                let hop = self.hop();
                st.conns
                    .get_mut(&from)
                    .expect("present")
                    .calls
                    .insert(call.id, Route::Remote { callee, hop });
                let callee_conn = st.conns.get_mut(&callee).expect("resolved");
                callee_conn.served.insert(
                    hop,
                    Served {
                        caller: Caller::Conn {
                            conn: from,
                            id: call.id,
                        },
                        kind: kind_of(&spec),
                        lane: lane_of(&spec),
                    },
                );
                Ok(Routed::Remote(
                    callee_conn.out.clone(),
                    Call {
                        id: hop,
                        from: identity,
                        ..call
                    },
                ))
            }
        }
    }

    /// Who answers `method` for a call naming `target` (empty: by method).
    fn resolve(&self, st: &mut State, method: &str, target: &str) -> Result<Callee, BusError> {
        if target == "tilrd" || (target.is_empty() && names::is_reserved(method)) {
            return self.native_method(method).map(Callee::Native);
        }
        let candidates: Vec<ConnId> = if target.is_empty() {
            st.providers
                .get(method)
                .into_iter()
                .flatten()
                .copied()
                .filter(|id| st.conns.get(id).is_some_and(|conn| !conn.draining))
                .collect()
        } else {
            let named: Vec<ConnId> = st
                .conns
                .iter()
                .filter(|(_, conn)| conn.identity == target)
                .map(|(id, _)| *id)
                .collect();
            if named.is_empty() {
                return Err(BusError::unavailable(format!("{target} is not connected")));
            }
            let providing: Vec<ConnId> = named
                .into_iter()
                .filter(|id| st.conns[id].provides.contains_key(method))
                .collect();
            if providing.is_empty() {
                return Err(BusError::new(
                    codes::NOT_FOUND,
                    format!("{target} does not provide {method}"),
                ));
            }
            providing
        };
        if candidates.is_empty() {
            return Err(BusError::unavailable(format!(
                "no provider of {method} is connected"
            )));
        }
        let spec_of = |st: &State, id: ConnId| st.conns[&id].provides[method].clone();
        let first = spec_of(st, candidates[0]);
        let chosen = match Select::try_from(first.select).unwrap_or(Select::RoundRobin) {
            Select::LastActive => *candidates
                .iter()
                .max_by_key(|id| (st.conns[id].activated, **id))
                .expect("not empty"),
            Select::RoundRobin => {
                let turn = st.turns.entry(method.to_owned()).or_default();
                let chosen = candidates[*turn % candidates.len()];
                *turn = turn.wrapping_add(1);
                chosen
            }
        };
        Ok(Callee::Conn(chosen, spec_of(st, chosen)))
    }

    /// Run a native method for the caller `from`.
    fn run_native(
        &self,
        from: ConnId,
        method: NativeMethod,
        ctx: CallCtx,
        payload: Bytes,
        generation: u64,
    ) {
        let id = ctx.id;
        let cancel = ctx.cancel.clone();
        let abort = match method.handler {
            NativeHandler::Unary(handler) => {
                let work = tokio::spawn(handler(ctx, payload));
                let abort = work.abort_handle();
                let broker = self.clone();
                tokio::spawn(async move {
                    let result = match work.await {
                        Ok(result) => result,
                        Err(err) if err.is_panic() => {
                            Err(BusError::handler_error("the handler panicked"))
                        }
                        // Cancelled: no answer.
                        Err(_) => return,
                    };
                    broker.finish_native(from, id, generation, result);
                });
                abort
            }
            NativeHandler::Stream(handler) => {
                let out = StreamOut {
                    broker: self.clone(),
                    conn: from,
                    id,
                    generation,
                    lane: lane_of(&method.spec),
                    cancel,
                    done: false,
                };
                tokio::spawn(handler(ctx, payload, out)).abort_handle()
            }
        };
        if let Some(conn) = self.lock().conns.get_mut(&from)
            && let Some(Route::Native {
                generation: current,
                abort: slot,
                ..
            }) = conn.calls.get_mut(&id)
            && *current == generation
        {
            *slot = Some(abort);
        }
    }

    /// Take the route of the native call `id` of `conn` if it is still the call of
    /// `generation` (not cancelled, nor replaced by a later call of the same id),
    /// with the caller's writer.
    fn take_native(&self, conn: ConnId, id: u64, generation: u64) -> Option<Out> {
        let mut st = self.lock();
        let caller = st.conns.get_mut(&conn)?;
        match caller.calls.get(&id) {
            Some(Route::Native {
                generation: current,
                ..
            }) if *current == generation => {
                caller.calls.remove(&id);
                Some(caller.out.clone())
            }
            _ => None,
        }
    }

    /// A native unary call is answered: unless it was cancelled meanwhile.
    fn finish_native(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        result: Result<Reply, BusError>,
    ) {
        let Some(out) = self.take_native(conn, id, generation) else {
            return;
        };
        let msg = match result {
            Ok(reply) => payload_reply(id, reply),
            Err(error) => error_reply(id, error),
        };
        out.send(msg, Lane::Interactive);
    }

    /// The caller's writer, while the native stream `id` of `generation` is open.
    fn native_stream_out(&self, conn: ConnId, id: u64, generation: u64) -> Option<Out> {
        let st = self.lock();
        let caller = st.conns.get(&conn)?;
        match caller.calls.get(&id) {
            Some(Route::Native {
                generation: current,
                ..
            }) if *current == generation => Some(caller.out.clone()),
            _ => None,
        }
    }

    pub(super) async fn stream_chunk(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        lane: Lane,
        chunk: Chunk,
    ) -> Result<(), Closed> {
        let out = self.native_stream_out(conn, id, generation).ok_or(Closed)?;
        let msg = Msg::Chunk(tilr_bus::bus::Chunk {
            id,
            payload: chunk.payload,
            content_encoding: chunk.content_encoding,
        });
        let sent = match lane {
            Lane::Bulk => out.send_bulk(msg).await,
            Lane::Interactive => out.send(msg, Lane::Interactive),
        };
        if sent { Ok(()) } else { Err(Closed) }
    }

    pub(super) fn try_stream_chunk(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        lane: Lane,
        chunk: Chunk,
    ) -> Result<(), TrySendError> {
        let Some(out) = self.native_stream_out(conn, id, generation) else {
            return Err(TrySendError::Closed);
        };
        if lane == Lane::Bulk && !out.idle() {
            return Err(TrySendError::Full(chunk));
        }
        let msg = Msg::Chunk(tilr_bus::bus::Chunk {
            id,
            payload: chunk.payload,
            content_encoding: chunk.content_encoding,
        });
        if out.send(msg, lane) {
            Ok(())
        } else {
            Err(TrySendError::Closed)
        }
    }

    /// The end of a native stream: `End` or an error `Reply`, behind its chunks on
    /// the stream's lane.
    pub(super) fn stream_end(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        lane: Lane,
        result: Result<Bytes, BusError>,
    ) {
        let Some(out) = self.take_native(conn, id, generation) else {
            return;
        };
        let msg = match result {
            Ok(payload) => Msg::End(End { id, payload }),
            Err(error) => error_reply(id, error),
        };
        out.send(msg, lane);
    }

    pub(super) fn conn_lagging(&self, conn: ConnId) -> bool {
        self.lock()
            .conns
            .get(&conn)
            .is_some_and(|conn| conn.out.lagging())
    }

    pub(super) async fn conn_caught_up(&self, conn: ConnId) {
        let out = self.lock().conns.get(&conn).map(|conn| conn.out.clone());
        if let Some(out) = out {
            out.caught_up().await;
        }
    }

    /// A served call's entry, taken (`remove`) or looked at.
    fn take_served(&self, callee: ConnId, hop: u64, remove: bool) -> Option<(Kind, Lane, Target)> {
        let mut st = self.lock();
        let conn = st.conns.get_mut(&callee)?;
        let (kind, lane) = {
            let served = conn.served.get(&hop)?;
            (served.kind, served.lane)
        };
        if !remove {
            let Caller::Conn { conn: caller, id } = conn.served[&hop].caller else {
                return Some((kind, lane, Target::None));
            };
            let out = st.conns.get(&caller).map(|c| c.out.clone());
            return Some((
                kind,
                lane,
                out.map_or(Target::None, |out| Target::Conn(out, id)),
            ));
        }
        let served = conn.served.remove(&hop).expect("present");
        match served.caller {
            Caller::Conn { conn: caller, id } => {
                let caller = st.conns.get_mut(&caller)?;
                caller.calls.remove(&id);
                Some((kind, lane, Target::Conn(caller.out.clone(), id)))
            }
            Caller::Native(tx) => Some((kind, lane, Target::Native(tx))),
        }
    }

    fn on_reply(&self, from: ConnId, reply: WireReply) {
        let Some((kind, lane, target)) = self.take_served(from, reply.id, true) else {
            return; // Late, cancelled or forged.
        };
        let bad =
            kind == Kind::Stream && matches!(reply.result, Some(reply::Result::Payload(_)) | None);
        match target {
            Target::Conn(out, id) => {
                let msg = if bad {
                    error_reply(
                        id,
                        BusError::bad_reply("a stream was answered with a reply instead of chunks"),
                    )
                } else {
                    Msg::Reply(WireReply { id, ..reply })
                };
                // A stream's end goes behind its chunks, on their lane.
                let lane = if kind == Kind::Stream {
                    lane
                } else {
                    Lane::Interactive
                };
                out.send(msg, lane);
            }
            Target::Native(tx) => {
                let _ = tx.send(wire_to_result(reply));
            }
            Target::None => {}
        }
    }

    fn on_chunk(&self, from: ConnId, chunk: tilr_bus::bus::Chunk) {
        let Some((kind, lane, Target::Conn(out, id))) = self.take_served(from, chunk.id, false)
        else {
            return;
        };
        if kind != Kind::Stream {
            return;
        }
        out.send(Msg::Chunk(tilr_bus::bus::Chunk { id, ..chunk }), lane);
    }

    fn on_end(&self, from: ConnId, end: End) {
        let Some((kind, lane, target)) = self.take_served(from, end.id, true) else {
            return;
        };
        if let Target::Conn(out, id) = target {
            let msg = if kind == Kind::Stream {
                Msg::End(End { id, ..end })
            } else {
                error_reply(id, BusError::bad_reply("a call was answered with End"))
            };
            // After the chunks, on their lane.
            out.send(msg, lane);
        }
    }

    fn on_cancel(&self, from: ConnId, cancel: Cancel) {
        let forward = {
            let mut st = self.lock();
            let Some(conn) = st.conns.get_mut(&from) else {
                return;
            };
            match conn.calls.remove(&cancel.id) {
                Some(Route::Remote { callee, hop }) => st.conns.get_mut(&callee).map(|callee| {
                    callee.served.remove(&hop);
                    Ok((callee.out.clone(), hop))
                }),
                Some(native) => Some(Err(native)),
                None => None,
            }
        };
        let forward = match forward {
            Some(Ok(forward)) => Some(forward),
            Some(Err(native)) => {
                native.stop();
                None
            }
            None => None,
        };
        if let Some((out, hop)) = forward {
            out.send(
                Msg::Cancel(Cancel {
                    id: hop,
                    reason: cancel.reason,
                }),
                Lane::Interactive,
            );
        }
    }

    fn on_credit(&self, from: ConnId, credit: Credit) {
        let forward = {
            let st = self.lock();
            match st
                .conns
                .get(&from)
                .and_then(|conn| conn.calls.get(&credit.id))
            {
                Some(Route::Remote { callee, hop }) => st
                    .conns
                    .get(callee)
                    .map(|callee| (callee.out.clone(), *hop)),
                _ => None,
            }
        };
        if let Some((out, hop)) = forward {
            out.send(Msg::Credit(Credit { id: hop, ..credit }), Lane::Interactive);
        }
    }

    fn on_subscribe(&self, from: ConnId, subscribe: Subscribe) {
        let Some((principal, out)) = self.principal(from) else {
            return;
        };
        let id = subscribe.id;
        if !names::valid_pattern(&subscribe.pattern) {
            out.send(
                error_reply(
                    id,
                    BusError::bad_request(format!("{:?} is not a pattern", subscribe.pattern)),
                ),
                Lane::Interactive,
            );
            return;
        }
        let allowed = self.0.authz.authorize(
            &principal,
            None,
            &AuthzRequest::Subscribe {
                pattern: &subscribe.pattern,
            },
        );
        let msg = match allowed {
            Err(denied) => {
                tracing::info!(target: "tilrd::bus::audit", connection = from, identity = %principal.identity, pattern = subscribe.pattern, "subscription denied: {denied}");
                error_reply(id, denied.into())
            }
            Ok(()) => {
                let added = self
                    .lock()
                    .conns
                    .get_mut(&from)
                    .is_some_and(|conn| conn.subscriptions.add(id, &subscribe.pattern));
                if added {
                    payload_reply(id, Reply::default())
                } else {
                    error_reply(
                        id,
                        BusError::bad_request(format!("subscription id {id} is in use")),
                    )
                }
            }
        };
        out.send(msg, Lane::Interactive);
    }

    fn on_event(&self, from: ConnId, event: WireEvent) {
        let Some((principal, _)) = self.principal(from) else {
            return;
        };
        if !names::valid_subject(&event.subject) {
            tracing::info!(target: "tilrd::bus::audit", connection = from, identity = %principal.identity, subject = event.subject, "dropped an event with a bad subject");
            return;
        }
        if names::is_reserved(&event.subject) {
            tracing::info!(target: "tilrd::bus::audit", connection = from, identity = %principal.identity, subject = event.subject, "dropped an event in a namespace of tilrd");
            return;
        }
        if let Err(denied) = self.0.authz.authorize(
            &principal,
            None,
            &AuthzRequest::Emit {
                subject: &event.subject,
            },
        ) {
            tracing::info!(target: "tilrd::bus::audit", connection = from, identity = %principal.identity, subject = event.subject, "dropped an event: {denied}");
            return;
        }
        // The scope is set only by the core.
        self.deliver(
            principal.identity.to_string(),
            event.subject,
            event.payload,
            event.content_type,
            String::new(),
        );
    }

    /// Emit an event of tilrd's core; `scope` (`pane:<id>`) limits who gets it.
    pub fn emit(&self, from: Identity, subject: &str, payload: Bytes, scope: Option<String>) {
        if !names::valid_subject(subject) {
            tracing::error!(target: "tilrd::bus", subject, "the core emitted a bad subject");
            return;
        }
        self.deliver(
            from.to_string(),
            subject.to_owned(),
            payload,
            String::new(),
            scope.unwrap_or_default(),
        );
    }

    /// Deliver an event to every connection subscribed to it now.
    fn deliver(
        &self,
        from: String,
        subject: String,
        payload: Bytes,
        content_type: String,
        scope: String,
    ) {
        let subscribers: Vec<(Arc<Principal>, Out)> = self
            .lock()
            .conns
            .values()
            .filter(|conn| conn.subscriptions.matches(&subject))
            .map(|conn| (Arc::clone(&conn.principal), conn.out.clone()))
            .collect();
        for (principal, out) in subscribers {
            let allowed = self.0.authz.authorize(
                &principal,
                None,
                &AuthzRequest::Event {
                    subject: &subject,
                    scope: &scope,
                },
            );
            if allowed.is_err() {
                continue;
            }
            out.send(
                Msg::Event(WireEvent {
                    subject: subject.clone(),
                    payload: payload.clone(),
                    content_type: content_type.clone(),
                    from: from.clone(),
                    scope: scope.clone(),
                }),
                Lane::Interactive,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Calls by tilrd's core
    // -----------------------------------------------------------------------

    /// Call a method of another participant as `from` (tilrd's core: `host.open`,
    /// `ssh_agent.*`). A `target` in `options` names the participant. No deadline
    /// in this change but `options.timeout`, measured here.
    pub async fn call(
        &self,
        from: Identity,
        method: &str,
        payload: impl Into<Bytes>,
        options: CallOptions,
    ) -> Result<Reply, BusError> {
        let target = options.target.clone().unwrap_or_default();
        let (tx, rx) = oneshot::channel();
        let hop = self.hop();
        let (out, call) = {
            let mut st = self.lock();
            if let Some(reason) = &st.stopped {
                return Err(BusError::unavailable(reason.clone()));
            }
            let callee = self.resolve(&mut st, method, &target)?;
            let Callee::Conn(callee, spec) = callee else {
                return Err(BusError::bad_request(format!(
                    "tilrd does not call itself ({method})"
                )));
            };
            let conn = st.conns.get_mut(&callee).expect("resolved");
            conn.served.insert(
                hop,
                Served {
                    caller: Caller::Native(tx),
                    kind: kind_of(&spec),
                    lane: lane_of(&spec),
                },
            );
            (
                conn.out.clone(),
                Call {
                    id: hop,
                    target,
                    method: method.to_owned(),
                    payload: payload.into(),
                    from: from.to_string(),
                    trace_id: options.trace_id.unwrap_or_default(),
                    timeout_ms: options.timeout.map_or(0, |timeout| {
                        timeout.as_millis().clamp(1, u32::MAX as u128) as u32
                    }),
                    content_type: options.content_type.unwrap_or_default(),
                    credential: Bytes::new(),
                    window: 0,
                },
            )
        };
        let callee_id = out.id();
        out.send(Msg::Call(call), Lane::Interactive);
        let answer = async {
            rx.await
                .unwrap_or_else(|_| Err(BusError::unavailable("the callee is gone")))
        };
        match options.timeout {
            None => answer.await,
            Some(timeout) => match tokio::time::timeout(timeout, answer).await {
                Ok(result) => result,
                Err(_) => {
                    let out = {
                        let mut st = self.lock();
                        st.conns
                            .get_mut(&callee_id)
                            .and_then(|conn| conn.served.remove(&hop).map(|_| conn.out.clone()))
                    };
                    if let Some(out) = out {
                        out.send(
                            Msg::Cancel(Cancel {
                                id: hop,
                                reason: "timeout".into(),
                            }),
                            Lane::Interactive,
                        );
                    }
                    Err(BusError::new(
                        codes::TIMEOUT,
                        format!("{method} did not answer within {} ms", timeout.as_millis()),
                    ))
                }
            },
        }
    }
}

enum Routed {
    Remote(Out, Call),
    Native(NativeMethod, CallCtx, Bytes, u64),
}

enum Target {
    Conn(Out, u64),
    Native(oneshot::Sender<Result<Reply, BusError>>),
    None,
}

fn conn_info(conn: &Conn) -> ConnInfo {
    let mut provides: Vec<MethodSpec> = conn.provides.values().cloned().collect();
    provides.sort_by(|a, b| a.method.cmp(&b.method));
    ConnInfo {
        connection_id: conn.principal.connection_id,
        identity: conn.identity.clone(),
        client_name: conn.principal.client_name.clone(),
        client_id: conn.principal.client_id.clone(),
        transport: conn.principal.transport,
        provides,
        draining: conn.draining,
    }
}

/// `bus.ping`, answered by the broker (B12): the nonce back.
pub(super) fn ping() -> NativeMethod {
    use prost::Message;
    use tilr_bus::bus::{PingReply, PingRequest};
    NativeMethod::unary(
        tilr_bus::methods::BUS_PING.name,
        |ctx, payload| async move {
            if ctx.wants_json() {
                let request: serde_json::Value = if payload.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_slice(&payload)
                        .map_err(|err| BusError::bad_request(format!("bus.ping: {err}")))?
                };
                // proto3 JSON: a uint64 is a string.
                let nonce = match &request["nonce"] {
                    serde_json::Value::String(text) => text.parse::<u64>().ok(),
                    serde_json::Value::Number(number) => number.as_u64(),
                    serde_json::Value::Null => Some(0),
                    _ => None,
                }
                .ok_or_else(|| BusError::bad_request("bus.ping: nonce is not a uint64"))?;
                let body = serde_json::json!({ "nonce": nonce.to_string() }).to_string();
                return Ok(Reply::new(body).with_content_type(tilr_bus::CONTENT_TYPE_JSON));
            }
            let request = PingRequest::decode(payload)
                .map_err(|err| BusError::bad_request(format!("bus.ping: {err}")))?;
            Ok(Reply::encode(&PingReply {
                nonce: request.nonce,
            }))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::authz::{AuthzConfig, default_authorizer};
    use crate::bus::identity::{Peer, Rights};
    use bytes::BytesMut;
    use tilr_bus::bus::Hello;
    use tilr_bus::codec;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    const CHUNK: usize = 32 << 10;

    /// A broker whose core has a bulk stream `test.count` (n chunks of 32 KiB),
    /// `test.hold` (keeps its `StreamOut` 100 ms past a cancel, then drops it) and
    /// `test.slow` (answers `second` after 300 ms).
    fn broker() -> BrokerHandle {
        let broker = BrokerHandle::new(BrokerOptions {
            authz: default_authorizer(&AuthzConfig::default()),
            server_id: "server-1".into(),
            next_connection_id: Arc::new(AtomicU64::new(1)),
            client_timeout: None,
        });
        broker.install(vec![
            NativeMethod::stream("test.count", Lane::Bulk, |_, payload, out| async move {
                let n: usize = String::from_utf8_lossy(&payload).parse().unwrap_or(0);
                for i in 0..n {
                    let body = Bytes::from(vec![i as u8; CHUNK]);
                    if out.send(Chunk::new(body)).await.is_err() {
                        return;
                    }
                }
                out.end(Bytes::from_static(b"done"));
            }),
            NativeMethod::stream("test.hold", Lane::Interactive, |_, _, out| async move {
                out.send(Chunk::new(Bytes::from_static(b"held")))
                    .await
                    .unwrap();
                // Outlives the handler, which the cancel aborts.
                tokio::spawn(async move {
                    out.cancelled().cancelled().await;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    drop(out);
                });
                std::future::pending::<()>().await;
            }),
            NativeMethod::unary("test.slow", |_, _| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(Reply::new("second"))
            }),
        ]);
        broker
    }

    /// An instance on a stream socket, greeted, with the right to call `test.*`.
    struct Peer0 {
        stream: UnixStream,
        buf: BytesMut,
    }

    impl Peer0 {
        async fn attach(broker: &BrokerHandle) -> Self {
            let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
            broker.attach_stream(
                ours.into(),
                Seat {
                    identity: Identity::Instance("t-1".into()),
                    rights: Some(Arc::new(Rights {
                        call: vec!["test.*".into()],
                        ..Rights::default()
                    })),
                    transport: Transport::InstanceFd,
                    peer: Peer::Child { pid: None },
                    check: None,
                },
            );
            theirs.set_nonblocking(true).unwrap();
            let mut peer = Self {
                stream: UnixStream::from_std(theirs).unwrap(),
                buf: BytesMut::new(),
            };
            peer.send(Msg::Hello(Hello {
                protocol_version: tilr_bus::PROTOCOL_VERSION,
                client_name: "t-1".into(),
                ..Hello::default()
            }))
            .await;
            assert!(matches!(peer.recv().await, Msg::HelloAck(_)));
            peer
        }

        async fn send(&mut self, msg: Msg) {
            let packet = codec::encode_stream(&Envelope { msg: Some(msg) }).unwrap();
            self.stream.write_all(&packet).await.unwrap();
        }

        async fn call(&mut self, id: u64, method: &str, payload: &'static str) {
            self.send(Msg::Call(Call {
                id,
                method: method.into(),
                target: "tilrd".into(),
                payload: Bytes::from_static(payload.as_bytes()),
                ..Call::default()
            }))
            .await;
        }

        async fn recv(&mut self) -> Msg {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Some(envelope) = codec::decode_stream(&mut self.buf).unwrap() {
                        if let Some(msg) = envelope.msg {
                            return msg;
                        }
                        continue;
                    }
                    let read = self.stream.read_buf(&mut self.buf).await.unwrap();
                    assert!(read > 0, "the broker closed the stream");
                }
            })
            .await
            .expect("nothing from the broker in time")
        }
    }

    #[tokio::test]
    async fn the_end_of_a_bulk_stream_comes_after_its_chunks() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        const CHUNKS: usize = 64;
        peer.call(1, "test.count", "64").await;
        let mut chunks = 0;
        loop {
            match peer.recv().await {
                Msg::Chunk(chunk) => {
                    assert_eq!(chunk.id, 1);
                    assert_eq!(chunk.payload.len(), CHUNK);
                    chunks += 1;
                }
                Msg::End(end) => {
                    assert_eq!(&end.payload[..], b"done");
                    break;
                }
                other => panic!("expected a chunk or End, got {other:?}"),
            }
        }
        assert_eq!(chunks, CHUNKS, "End came before the last chunk");
    }

    #[tokio::test]
    async fn a_late_end_of_a_cancelled_call_leaves_a_new_call_of_its_id_alone() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        peer.call(5, "test.hold", "").await;
        assert!(matches!(peer.recv().await, Msg::Chunk(chunk) if chunk.id == 5));
        // Cancel and reuse the id at once; the old stream's provider lets go of it
        // 100 ms later, while the new call is still running.
        peer.send(Msg::Cancel(Cancel {
            id: 5,
            reason: "enough".into(),
        }))
        .await;
        peer.call(5, "test.slow", "").await;
        match peer.recv().await {
            Msg::Reply(WireReply {
                id: 5,
                result: Some(reply::Result::Payload(payload)),
                ..
            }) => assert_eq!(&payload[..], b"second"),
            other => panic!("expected the new call's reply, got {other:?}"),
        }
    }
}
