//! The broker: connections, the routing table, pending calls and streams (design
//! B1, B2, B4-B8, B11, B12). Owner: `bus-broker-core`, then `bus-streams-events`
//! (bounds, credits, deadlines), then `bus-reload`.
//!
//! - A `Call` with a `target` goes to that participant; one without goes to a live,
//!   non-draining provider of the method by its `Select` (in turn in connection
//!   order, or the provider that sent `Activate` last, else the last connected).
//!   The namespaces of [`tilr_bus::names::RESERVED_NAMESPACES`] are the native
//!   participant `tilrd`'s. A method is known while a connection provides it,
//!   or when the configuration names it, a client method or a spec method
//!   ([`SEEDED_METHODS`]); a known method with no live, non-draining provider
//!   is `unavailable`, any other `not_found`. Names a connection added are
//!   forgotten when no connection provides them.
//! - `from` is the sender's identity, whatever it wrote. Call ids are remapped per
//!   hop, so a `Reply`, `Chunk` or `End` is taken only from the connection the call
//!   went to; a late or forged one is dropped.
//! - Envelopes of one connection are handled in the order read and queued in the
//!   callee's lanes in that order: calls (and cancels) between two connections keep
//!   their order. The native participant's handler functions are called in that
//!   order too ([`super::NativeHandler`]).
//! - Bounds (B5, B8): a connection has at most `max_in_flight` (256) calls and
//!   streams pending (calls it made plus answered streams whose chunks still
//!   wait), more are answered `busy`; a callee whose interactive lane holds
//!   [`OPTIONAL_ROOM`](super::transport::OPTIONAL_ROOM) envelopes or
//!   [`OPTIONAL_BYTES`](super::transport::OPTIONAL_BYTES) gets no new call
//!   (`busy`). A lane past its bound in bytes closes the connection `4001`
//!   ([`super::transport`]). A call's deadline is `min(timeout_ms, call_timeout_ms)`
//!   (a stream's: its `timeout_ms`, none by default); the callee is told the time
//!   left in `timeout_ms`, and at the deadline the caller gets `timeout` and the
//!   callee `Cancel`. The deadlines sit in one timer wheel ([`Timers`]), removed
//!   with the answer.
//! - A stream is a call to a `STREAM` method: `Chunk*`, then `End` or an error
//!   `Reply`; a `Reply` with a payload becomes `bad_reply`. Its producer sends at
//!   most the credit its reader gave (`Call.window`, then `Credit`s); one that
//!   sends more gets `Cancel` and its reader `bad_reply` (B7). The caller's
//!   `Cancel` is forwarded and later chunks are dropped. Credit counts
//!   `Chunk.payload` as sent (compressed when `content_encoding` says so), as
//!   `tilr-bus` does. An empty chunk is relayed like any other; held, it costs
//!   [`CHUNK_COST`] like any tiny chunk.
//! - Bulk streams to one connection have at most `connection_window` (1 MiB) of
//!   payload sent and not yet credited by it; the rest waits: a native producer in
//!   [`StreamOut::send`], a relayed chunk in the connection's held queue. The
//!   broker lets the provider of a relayed bulk stream send at most
//!   [`RELAY_WINDOW`] ahead of what reached the reader (it forwards a smaller
//!   window and grants credit as chunks go on), and at most [`RELAY_BUDGET`] for
//!   all relayed bulk streams to one reader; a stream past it is `busy`. A held
//!   chunk costs at least [`CHUNK_COST`], so a flood of tiny chunks ends its
//!   stream `bad_reply`. A chunk that cannot be re-encoded for its reader ends
//!   the stream. The end of a stream (`End`, an error, `unavailable`, `timeout`)
//!   goes behind its chunks, on their lane, and needs no credit.
//! - Events go to a subscriber's interactive lane unless that lane lags (a packet
//!   held over 250 ms, or `OPTIONAL_ROOM` queued): then they are dropped and
//!   counted (`tilrd_bus_events_dropped_total`, [`super::events`]), and the
//!   connection stays (B6).
//! - A connection's end answers the calls sent to it `unavailable` and cancels the
//!   calls it made.
//!
//! The state is one mutex, held for bookkeeping only: never across an await or
//! an authorization, and payloads (chunks, calls, replies, a stream's `End`) are
//! encoded outside it; only small error envelopes the broker makes are encoded
//! under it, so that they stay behind the chunks of their stream. Letting go of
//! it wakes the drains waiting for the calls to be answered. Design
//! "Deviations": why it is not split per connection.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use bytes::Bytes;
use tilr_bus::bus::{
    Call, Cancel, ConnectionError, Credit, End, Envelope, Error as WireError, Event as WireEvent,
    Kind, Lane, MethodSpec, Reply as WireReply, Select, Subscribe, Unsubscribe, envelope::Msg,
    reply,
};
use tilr_bus::{BusError, CallOptions, Reply, codes, limits, names};
use tokio::sync::{Notify, oneshot, watch};
use tokio::time::Instant;

use super::authz::{Authorizer, AuthzRequest};
use super::events::{self, Subscriptions};
use super::identity::{CallCtx, CancelToken, Identity, Principal, Seat, Transport};
use super::streams::{Chunk, Closed, NativeHandler, NativeMethod, StreamOut, TrySendError};
use super::transport::{Offer, Out};

/// The broker's id of a connection, `HelloAck.connection_id`: from the counter the
/// `tilr.v1` connections take theirs from, so it grows across reloads.
pub type ConnId = u64;

/// Calls and streams pending per caller (`HelloAck.max_in_flight`).
const MAX_IN_FLIGHT: usize = limits::MAX_IN_FLIGHT as usize;
/// Bulk payload bytes sent to a connection and not yet credited by it
/// (`HelloAck.connection_window`).
const CONNECTION_WINDOW: u64 = limits::CONNECTION_WINDOW as u64;
/// What the provider of a relayed bulk stream may send ahead of what reached its
/// reader: the broker's memory for one such stream.
pub const RELAY_WINDOW: u64 = CONNECTION_WINDOW;
/// What the broker holds and has granted, at most, of the relayed bulk streams
/// one connection reads (counted as [`CHUNK_COST`] per chunk at least): a
/// stream that would go past it is not opened (`busy`), a grant waits for room.
pub const RELAY_BUDGET: u64 = 16 << 20;
/// The least a held chunk counts in [`RELAY_BUDGET`]: small chunks do not pile
/// up by the million. Never counted in the credit the producer and the reader
/// see, which is payload bytes.
pub const CHUNK_COST: u64 = 1 << 10;
/// Methods the spec gives to windows and plugins (design W1, B11) and the ones
/// the core calls on clients: without a provider they are `unavailable` from the
/// start.
const SEEDED_METHODS: &[&str] = &["ui.notify", "ui.pick_file"];

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
    /// Woken each time the state's lock is let go: what a drain waits on.
    changed: Notify,
    /// `call_timeout_ms` (bus.toml); 0: no default deadline.
    call_timeout_ms: AtomicU32,
    timers: Arc<Timers>,
}

impl Drop for Shared {
    fn drop(&mut self) {
        // The timer task sees the broker gone and ends.
        self.timers.wake.notify_one();
    }
}

/// The state's lock; letting go of it wakes what waits on [`Shared::changed`]
/// (the drains), so they wait for the calls to be answered instead of polling.
struct StateGuard<'a> {
    guard: MutexGuard<'a, State>,
    changed: &'a Notify,
}

impl std::ops::Deref for StateGuard<'_> {
    type Target = State;
    fn deref(&self) -> &State {
        &self.guard
    }
}

impl std::ops::DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.guard
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        // Cheap without waiters (an atomic load).
        self.changed.notify_waiters();
    }
}

/// When a call's time is up: the deadline, the caller, its id of the call, and the
/// hop or generation that tells this call from a later one of the same id.
type TimerKey = (Instant, ConnId, u64, u64);

/// The deadlines of the calls in flight, in one ordered set served by one task
/// (B8's timer wheel): armed when a call is routed, removed with its answer.
#[derive(Default)]
struct Timers {
    due: Mutex<BTreeSet<TimerKey>>,
    wake: Notify,
    started: OnceLock<()>,
}

impl Timers {
    fn due(&self) -> MutexGuard<'_, BTreeSet<TimerKey>> {
        self.due
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

async fn run_timers(timers: Arc<Timers>, shared: Weak<Shared>) {
    loop {
        let wake = timers.wake.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let now = Instant::now();
        let (expired, next) = {
            let mut due = timers.due();
            let mut expired = Vec::new();
            while let Some(first) = due.first().copied() {
                if first.0 > now {
                    break;
                }
                due.pop_first();
                expired.push(first);
            }
            (expired, due.first().map(|key| key.0))
        };
        if !expired.is_empty() {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            let broker = BrokerHandle(shared);
            for key in expired {
                broker.expire(key);
            }
            continue;
        }
        if shared.strong_count() == 0 {
            return;
        }
        match next {
            Some(at) => {
                tokio::select! {
                    () = tokio::time::sleep_until(at) => {}
                    () = &mut wake => {}
                }
            }
            None => wake.await,
        }
    }
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
    /// Methods an instance of bus.toml will provide ([`BrokerHandle::know_methods`]):
    /// without a provider they are `unavailable`. So are the methods provided
    /// now and [`SEEDED_METHODS`]; any other is `not_found`.
    configured: HashSet<String>,
}

impl State {
    /// Whether a call of `method` without a provider is `unavailable` rather
    /// than `not_found`.
    fn known(&self, method: &str) -> bool {
        self.providers.contains_key(method)
            || self.configured.contains(method)
            || SEEDED_METHODS.contains(&method)
            || tilr_bus::methods::CLIENT
                .iter()
                .any(|client| client.name == method)
    }
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
    /// The streams this connection reads, by its id: their credit and what the
    /// broker holds of them. A stream's flow outlives its route while its end
    /// waits behind held chunks.
    flows: HashMap<u64, Flow>,
    /// The relayed bulk streams with chunks (or an end) held, in turn: one
    /// entry per such stream (`Flow::queued`); an entry of a stream that is gone
    /// is skipped.
    ready: VecDeque<(u64, u64)>,
    /// Bulk payload bytes sent to this connection on its open streams and not yet
    /// credited by it (B7: at most [`CONNECTION_WINDOW`]).
    bulk_uncredited: u64,
    /// What the broker holds and has granted of the relayed bulk streams this
    /// connection reads: the sum of their `avail + held_cost` (at most
    /// [`RELAY_BUDGET`]).
    relay_outstanding: u64,
    /// A grant was cut short by [`RELAY_BUDGET`]: the streams are granted again
    /// when room comes back.
    relay_starved: bool,
    /// Woken when credit or window comes back, a stream of it ends, or the
    /// connection goes: the native producers wait on it.
    moved: Arc<Notify>,
    state: watch::Sender<ConnState>,
}

impl Conn {
    /// Forget the stream `id` of `unique` for its reader: what of it the broker
    /// holds is dropped (with the flow, not searched for) and its uncredited
    /// bytes go back to the window. [`pump`] grants the room it frees.
    fn drop_flow(&mut self, id: u64, unique: u64) {
        if !self
            .flows
            .get(&id)
            .is_some_and(|flow| flow.unique == unique)
        {
            return;
        }
        if let Some(flow) = self.flows.remove(&id) {
            self.bulk_uncredited = self.bulk_uncredited.saturating_sub(flow.uncredited);
            if flow.relayed_bulk() {
                self.relay_outstanding = self
                    .relay_outstanding
                    .saturating_sub(flow.avail + flow.held_cost);
            }
        }
        self.moved.notify_waiters();
    }

    /// Streams this connection still reads whose call is over: a relayed bulk
    /// stream whose end waits behind held chunks. They count as pending.
    fn lingering(&self) -> usize {
        self.flows
            .keys()
            .filter(|id| !self.calls.contains_key(id))
            .count()
    }
}

/// Where a pending call went.
struct Route {
    to: To,
    method: String,
    /// Its deadline in the timer wheel, and the timeout in milliseconds.
    timer: Option<(TimerKey, u32)>,
}

enum To {
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
    /// The hop or generation of this call.
    fn unique(&self) -> u64 {
        match &self.to {
            To::Remote { hop, .. } => *hop,
            To::Native { generation, .. } => *generation,
        }
    }

    fn is_native(&self, generation: u64) -> bool {
        matches!(self.to, To::Native { generation: current, .. } if current == generation)
    }

    /// Stop a native call's handler (outside the lock).
    fn stop(self) {
        if let To::Native { cancel, abort, .. } = self.to {
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
    /// The `Call` is queued for the callee. Until it is, a `Cancel` of it is not
    /// sent (it would come first); whoever queues the call sends it after.
    offered: bool,
}

enum Caller {
    Conn {
        conn: ConnId,
        id: u64,
    },
    /// tilrd's core, waiting in [`BrokerHandle::call`].
    Native(oneshot::Sender<Result<Reply, BusError>>),
}

/// The credit of one stream, on its reader's connection (B7).
struct Flow {
    /// The hop (relayed) or generation (native) of its call.
    unique: u64,
    lane: Lane,
    /// Relayed from a participant (not produced by tilrd).
    relayed: bool,
    /// The window its reader opened it with: no chunk of a native producer is
    /// larger.
    window: u64,
    /// What the producer may send now: the reader's credit, or, for a relayed
    /// bulk stream, what the broker granted the provider.
    avail: u64,
    /// Relayed bulk: what the reader allows beyond what reached it.
    reader_avail: u64,
    /// Relayed bulk: its chunks the broker holds, encoded for the reader, in order.
    held: VecDeque<HeldChunk>,
    /// Relayed bulk: what `held` counts in [`RELAY_BUDGET`].
    held_cost: u64,
    /// Relayed bulk: its end (`End`, an error), encoded, behind `held`.
    end: Option<Bytes>,
    /// Relayed bulk: it has its entry in [`Conn::ready`].
    queued: bool,
    /// Bulk: bytes sent to the reader and not yet credited by it.
    uncredited: u64,
    /// Relayed, while its provider may still send: the provider and the hop.
    provider: Option<(ConnId, u64)>,
}

impl Flow {
    fn new(
        unique: u64,
        lane: Lane,
        window: u64,
        avail: u64,
        provider: Option<(ConnId, u64)>,
    ) -> Self {
        Self {
            unique,
            lane,
            relayed: provider.is_some(),
            window,
            avail,
            reader_avail: window,
            held: VecDeque::new(),
            held_cost: 0,
            end: None,
            queued: false,
            uncredited: 0,
            provider,
        }
    }

    fn relayed_bulk(&self) -> bool {
        self.relayed && self.lane == Lane::Bulk
    }
}

/// A relayed bulk chunk, encoded for its reader.
struct HeldChunk {
    packet: Bytes,
    /// Its payload bytes.
    bytes: u64,
    /// What it counts in [`RELAY_BUDGET`]: `max(bytes, CHUNK_COST)`.
    cost: u64,
}

/// What a chunk of `bytes` counts in the broker's own bounds.
fn chunk_cost(bytes: u64) -> u64 {
    bytes.max(CHUNK_COST)
}

/// Credit the broker gives the provider of a relayed bulk stream.
struct Grant {
    provider: ConnId,
    hop: u64,
    bytes: u64,
}

/// Credit the provider of a relayed bulk stream may get now: up to what the
/// reader allows, [`RELAY_WINDOW`] ahead of what reached it, and what is left of
/// the reader's [`RELAY_BUDGET`] (`outstanding`); `starved` is set when the
/// budget cut it short.
fn grant(flow: &mut Flow, outstanding: &mut u64, starved: &mut bool) -> Option<Grant> {
    if !flow.relayed_bulk() {
        return None;
    }
    let (provider, hop) = flow.provider?;
    let target = flow.reader_avail.min(RELAY_WINDOW);
    let have = flow.avail + flow.held_cost;
    if target <= have {
        return None;
    }
    let want = target - have;
    let bytes = want.min(RELAY_BUDGET.saturating_sub(*outstanding));
    if bytes < want {
        *starved = true;
    }
    if bytes == 0 {
        return None;
    }
    flow.avail += bytes;
    *outstanding += bytes;
    Some(Grant {
        provider,
        hop,
        bytes,
    })
}

/// Forward what `c` holds while its connection window and its bulk lane let it,
/// a chunk of each held stream in turn; the providers' grants go to `grants`.
/// Called with the lock held: the chunks of a stream are queued in order
/// whoever pumps.
fn pump(c: &mut Conn, grants: &mut Vec<Grant>) {
    while let Some(&(id, unique)) = c.ready.front() {
        let Some(flow) = c
            .flows
            .get_mut(&id)
            .filter(|flow| flow.unique == unique && flow.queued)
        else {
            c.ready.pop_front(); // Its stream is gone.
            continue;
        };
        let Some(front) = flow.held.front() else {
            // Only its end is left.
            c.ready.pop_front();
            flow.queued = false;
            if let Some(end) = flow.end.take() {
                c.drop_flow(id, unique);
                c.out.send_packet(end, Lane::Bulk);
            }
            continue;
        };
        let bytes = front.bytes;
        if c.bulk_uncredited > 0 && c.bulk_uncredited + bytes > CONNECTION_WINDOW {
            break;
        }
        if !c.out.bulk_room(front.packet.len()) {
            break; // The writer asks again ([`BrokerHandle::bulk_room`]).
        }
        let Some(chunk) = flow.held.pop_front() else {
            break;
        };
        flow.held_cost -= chunk.cost;
        c.relay_outstanding = c.relay_outstanding.saturating_sub(chunk.cost);
        flow.reader_avail = flow.reader_avail.saturating_sub(bytes);
        flow.uncredited += bytes;
        c.bulk_uncredited += bytes;
        grants.extend(grant(flow, &mut c.relay_outstanding, &mut c.relay_starved));
        c.ready.pop_front();
        if flow.held.is_empty() && flow.end.is_none() {
            flow.queued = false;
        } else {
            c.ready.push_back((id, unique));
        }
        c.out.send_packet(chunk.packet, Lane::Bulk);
    }
    // Entries of streams that are gone, behind a stalled one.
    if c.ready.len() > 2 * c.flows.len() + 16 {
        let flows = &c.flows;
        c.ready.retain(|(id, unique)| {
            flows
                .get(id)
                .is_some_and(|flow| flow.unique == *unique && flow.queued)
        });
    }
    if c.relay_starved {
        c.relay_starved = false;
        for flow in c.flows.values_mut() {
            grants.extend(grant(flow, &mut c.relay_outstanding, &mut c.relay_starved));
        }
    }
}

/// The sends that carry `grants`.
fn grant_sends(st: &State, grants: Vec<Grant>) -> Vec<(Out, Msg)> {
    grants
        .into_iter()
        .filter_map(|grant| {
            let out = st.conns.get(&grant.provider)?.out.clone();
            Some((
                out,
                Msg::Credit(Credit {
                    id: grant.hop,
                    bytes: u32::try_from(grant.bytes).unwrap_or(u32::MAX),
                }),
            ))
        })
        .collect()
}

/// The last word to the caller `conn` on its call `id` of `unique`, encoded for
/// it: an answer of a unary call goes now, on the interactive lane; the end of a
/// stream (`End`, an error) behind the chunks of it the broker holds, on the
/// stream's lane. Never waits for credit.
fn answer(
    st: &mut State,
    conn: ConnId,
    id: u64,
    unique: u64,
    packet: Bytes,
    grants: &mut Vec<Grant>,
) {
    let Some(c) = st.conns.get_mut(&conn) else {
        return;
    };
    let Some(flow) = c.flows.get_mut(&id).filter(|flow| flow.unique == unique) else {
        c.out.send_packet(packet, Lane::Interactive);
        return;
    };
    // Its producer sends no more: what it was granted is free.
    flow.provider = None;
    if flow.relayed_bulk() {
        c.relay_outstanding = c.relay_outstanding.saturating_sub(flow.avail);
    }
    flow.avail = 0;
    if !flow.held.is_empty() {
        flow.end = Some(packet);
        return;
    }
    let lane = flow.lane;
    c.drop_flow(id, unique);
    c.out.send_packet(packet, lane);
    pump(c, grants);
}

/// [`answer`] with a small envelope (an error of the broker), encoded here.
fn answer_msg(
    st: &mut State,
    conn: ConnId,
    id: u64,
    unique: u64,
    msg: Msg,
    grants: &mut Vec<Grant>,
) {
    let Some(packet) = st.conns.get(&conn).and_then(|c| c.out.encode(msg)) else {
        return;
    };
    answer(st, conn, id, unique, packet, grants);
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

/// A stream's initial credit: `Call.window`, 0 the default, at most 16 MiB.
fn stream_window(method: &str, window: u32) -> Result<u64, BusError> {
    match window {
        0 => Ok(u64::from(limits::STREAM_WINDOW)),
        window if window > limits::MAX_STREAM_WINDOW => Err(BusError::bad_request(format!(
            "{method}: a window of {window} is over the limit of {}",
            limits::MAX_STREAM_WINDOW
        ))),
        window => Ok(u64::from(window)),
    }
}

fn timed_out(method: &str, ms: u32) -> BusError {
    BusError::new(
        codes::TIMEOUT,
        format!("{method} did not answer within {ms} ms"),
    )
}

fn busy(message: String) -> BusError {
    BusError::new(codes::BUSY, message)
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
            changed: Notify::new(),
            call_timeout_ms: AtomicU32::new(limits::CALL_TIMEOUT_MS),
            timers: Arc::new(Timers::default()),
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

    /// The deadline of a call that names none, and the most a call names
    /// (`call_timeout_ms` of bus.toml; 10 s until it is read). Zero: no default
    /// deadline.
    pub fn set_call_timeout(&self, timeout: Duration) {
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        self.0.call_timeout_ms.store(ms, Ordering::Relaxed);
    }

    /// Methods a participant will provide (an instance in bus.toml): calls of
    /// them without a provider are `unavailable`, not `not_found`. Kept whole
    /// (bus.toml is the owner's).
    pub fn know_methods(&self, methods: impl IntoIterator<Item = String>) {
        self.lock().configured.extend(methods);
    }

    /// Send a connection a `CredentialUpdate` (an instance's renewed or rotated
    /// token); `false` if it is not connected.
    pub fn send_credential_update(&self, conn: ConnId, credential: Bytes) -> bool {
        let out = self.lock().conns.get(&conn).map(|c| c.out.clone());
        out.is_some_and(|out| {
            out.send(
                Msg::CredentialUpdate(tilr_bus::bus::CredentialUpdate { credential }),
                Lane::Interactive,
            )
        })
    }

    /// How a greeted connection is doing, to be told when it ends
    /// ([`ConnState::Ended`]); `None` if it is not connected now.
    pub fn watch_connection(&self, conn: ConnId) -> Option<watch::Receiver<ConnState>> {
        self.lock().conns.get(&conn).map(|c| c.state.subscribe())
    }

    /// A new connection's id.
    pub fn next_connection_id(&self) -> ConnId {
        self.0.next_connection_id.fetch_add(1, Ordering::Relaxed)
    }

    fn lock(&self) -> StateGuard<'_> {
        StateGuard {
            guard: self
                .0
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            changed: &self.0.changed,
        }
    }

    /// Wait until `done` holds of the state, woken by each change of it (no
    /// polling), or until `deadline`; whether it holds.
    async fn wait_state(
        &self,
        deadline: tokio::time::Instant,
        done: impl Fn(&State) -> bool,
    ) -> bool {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if done(&self.lock()) {
                return true;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return done(&self.lock());
            }
        }
    }

    fn hop(&self) -> u64 {
        self.0.next_hop.fetch_add(1, Ordering::Relaxed)
    }

    /// The deadline of a call that gives `timeout_ms`: `min(timeout_ms,
    /// call_timeout_ms)`, 0 the default; a stream has none unless it gives one.
    fn timeout_ms(&self, timeout_ms: u32, stream: bool) -> Option<u32> {
        let default = self.0.call_timeout_ms.load(Ordering::Relaxed);
        match (timeout_ms, stream) {
            (0, true) => None,
            (0, false) => (default > 0).then_some(default),
            (ms, true) => Some(ms),
            (ms, false) if default > 0 => Some(ms.min(default)),
            (ms, false) => Some(ms),
        }
    }

    /// Put a deadline in the timer wheel (the wheel's task starts with the first).
    fn arm(&self, key: TimerKey) {
        let timers = &self.0.timers;
        let first = {
            let mut due = timers.due();
            due.insert(key);
            due.first() == Some(&key)
        };
        timers.started.get_or_init(|| {
            tokio::spawn(run_timers(Arc::clone(timers), Arc::downgrade(&self.0)));
        });
        if first {
            timers.wake.notify_one();
        }
    }

    /// Take a call's deadline out of the timer wheel: it was answered.
    fn disarm(&self, route: &Route) {
        if let Some((key, _)) = route.timer {
            self.0.timers.due().remove(&key);
        }
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
                flows: HashMap::new(),
                ready: VecDeque::new(),
                bulk_uncredited: 0,
                relay_outstanding: 0,
                relay_starved: false,
                moved: Arc::new(Notify::new()),
                state,
            },
        );
    }

    /// Forget a connection: the calls sent to it are answered `unavailable` (a
    /// stream's behind its chunks), the calls it made are cancelled. Returns its
    /// writer, for the caller to close.
    pub(super) fn disconnect(&self, id: ConnId) -> Option<Out> {
        let mut sends: Vec<(Out, Msg)> = Vec::new();
        let mut natives: Vec<oneshot::Sender<Result<Reply, BusError>>> = Vec::new();
        let mut stopped: Vec<Route> = Vec::new();
        let conn = {
            let mut st = self.lock();
            let mut conn = st.conns.remove(&id)?;
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
            let mut grants = Vec::new();
            for (hop, served) in std::mem::take(&mut conn.served) {
                match served.caller {
                    Caller::Conn { conn: caller, id } => {
                        if self.end_route(&mut st, caller, id, hop).is_some() {
                            answer_msg(
                                &mut st,
                                caller,
                                id,
                                hop,
                                error_reply(id, BusError::unavailable(gone.clone())),
                                &mut grants,
                            );
                        }
                    }
                    Caller::Native(tx) => natives.push(tx),
                }
            }
            for (_, route) in conn.calls.drain() {
                self.disarm(&route);
                if let To::Remote { callee, hop } = route.to {
                    if let Some(callee) = st.conns.get_mut(&callee)
                        && callee
                            .served
                            .remove(&hop)
                            .is_some_and(|served| served.offered)
                    {
                        sends.push((
                            callee.out.clone(),
                            Msg::Cancel(Cancel {
                                id: hop,
                                reason: "the caller's connection ended".into(),
                            }),
                        ));
                    }
                } else {
                    stopped.push(route);
                }
            }
            sends.extend(grant_sends(&st, grants));
            conn
        };
        conn.moved.notify_waiters();
        for route in stopped {
            route.stop();
        }
        let Conn {
            out,
            state,
            identity,
            ..
        } = conn;
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

    /// Take the route of the call `id` of `caller` if it is the call of `unique`,
    /// with its deadline out of the wheel.
    fn end_route(&self, st: &mut State, caller: ConnId, id: u64, unique: u64) -> Option<Route> {
        let conn = st.conns.get_mut(&caller)?;
        if !conn
            .calls
            .get(&id)
            .is_some_and(|route| route.unique() == unique)
        {
            return None;
        }
        let route = conn.calls.remove(&id)?;
        self.disarm(&route);
        Some(route)
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
    /// answer the rest `unavailable` (a stream's behind its chunks) and cancel them
    /// at it. Returns whether every call sent to it was answered in time. The
    /// connection stays open.
    pub async fn drain_connection(&self, id: ConnId, timeout: Duration) -> bool {
        self.set_draining(id, true);
        let deadline = tokio::time::Instant::now() + timeout;
        let answered = |st: &State| st.conns.get(&id).is_none_or(|conn| conn.served.is_empty());
        if self.wait_state(deadline, answered).await {
            return true;
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
            let mut grants = Vec::new();
            for (hop, served) in served {
                if served.offered {
                    sends.push((
                        callee_out.clone(),
                        Msg::Cancel(Cancel {
                            id: hop,
                            reason: reason.into(),
                        }),
                    ));
                }
                match served.caller {
                    Caller::Conn { conn: caller, id } => {
                        if self.end_route(&mut st, caller, id, hop).is_some() {
                            answer_msg(
                                &mut st,
                                caller,
                                id,
                                hop,
                                error_reply(id, BusError::unavailable(reason)),
                                &mut grants,
                            );
                        }
                    }
                    Caller::Native(tx) => natives.push(tx),
                }
            }
            sends.extend(grant_sends(&st, grants));
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
    /// `unavailable` (a stream's behind its chunks) and cancel them at their
    /// callees.
    pub async fn drain(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        let settled = |st: &State| {
            st.conns
                .values()
                .all(|conn| conn.calls.is_empty() && conn.served.is_empty())
        };
        self.wait_state(deadline, settled).await;
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
            let mut grants = Vec::new();
            let ids: Vec<ConnId> = st.conns.keys().copied().collect();
            for id in ids {
                let Some(conn) = st.conns.get_mut(&id) else {
                    continue;
                };
                let calls: Vec<(u64, Route)> = conn.calls.drain().collect();
                for (call_id, route) in calls {
                    self.disarm(&route);
                    let unique = route.unique();
                    match route.to {
                        To::Remote { callee, hop } => {
                            if let Some(callee) = st.conns.get_mut(&callee)
                                && callee
                                    .served
                                    .remove(&hop)
                                    .is_some_and(|served| served.offered)
                            {
                                sends.push((
                                    callee.out.clone(),
                                    Msg::Cancel(Cancel {
                                        id: hop,
                                        reason: reason.clone(),
                                    }),
                                ));
                            }
                        }
                        To::Native { .. } => stopped.push(route),
                    }
                    answer_msg(
                        &mut st,
                        id,
                        call_id,
                        unique,
                        error_reply(call_id, BusError::unavailable(reason.clone())),
                        &mut grants,
                    );
                }
            }
            for conn in st.conns.values_mut() {
                for (_, served) in conn.served.drain() {
                    if let Caller::Native(tx) = served.caller {
                        natives.push(tx);
                    }
                }
            }
            sends.extend(grant_sends(&st, grants));
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
            Ok(Routed::Remote {
                callee,
                callee_out,
                identity,
                hop,
                call,
            }) => {
                let method = call.method.clone();
                let refusal = match callee_out.encode(Msg::Call(call)) {
                    None => BusError::new(
                        codes::TOO_LARGE,
                        format!("{method}: the call is over the frame limit"),
                    ),
                    Some(packet) => match callee_out.offer(packet, false) {
                        Offer::Queued => {
                            // Queued: a Cancel of it may go now. Ended meanwhile
                            // (a deadline, the caller gone): its Cancel goes
                            // after it.
                            let ended = {
                                let mut st = self.lock();
                                match st
                                    .conns
                                    .get_mut(&callee)
                                    .and_then(|conn| conn.served.get_mut(&hop))
                                {
                                    Some(served) => {
                                        served.offered = true;
                                        false
                                    }
                                    None => true,
                                }
                            };
                            if ended {
                                callee_out.send(
                                    Msg::Cancel(Cancel {
                                        id: hop,
                                        reason: "the call ended before it was sent".into(),
                                    }),
                                    Lane::Interactive,
                                );
                            }
                            return;
                        }
                        Offer::Full => busy(format!(
                            "{method} not sent: {identity} is busy, its queue is full"
                        )),
                        Offer::Gone => BusError::unavailable(format!("{identity} disconnected")),
                    },
                };
                // Undone: the callee never got it. Answered only if nothing else
                // answered it meanwhile (a deadline, the callee gone).
                let (refuse, sends) = {
                    let mut st = self.lock();
                    if let Some(conn) = st.conns.get_mut(&callee) {
                        conn.served.remove(&hop);
                    }
                    let refuse = self.end_route(&mut st, from, id, hop).is_some();
                    let mut grants = Vec::new();
                    if let Some(conn) = st.conns.get_mut(&from) {
                        conn.drop_flow(id, hop);
                        pump(conn, &mut grants);
                    }
                    (refuse, grant_sends(&st, grants))
                };
                if refuse {
                    out.send(error_reply(id, refusal), Lane::Interactive);
                }
                self.send_all(sends);
            }
            Ok(Routed::Native(method, ctx, payload, generation, window)) => {
                self.run_native(from, method, ctx, payload, generation, window);
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
        if caller.calls.contains_key(&call.id) || caller.flows.contains_key(&call.id) {
            return Err(BusError::bad_request(format!(
                "call id {} is in use on this connection",
                call.id
            )));
        }
        // Streams whose end waits behind held chunks count as pending.
        if caller.calls.len() + caller.lingering() >= MAX_IN_FLIGHT {
            return Err(busy(format!(
                "{} not sent: {MAX_IN_FLIGHT} calls of this connection are pending",
                call.method
            )));
        }
        let callee = self.resolve(st, &call.method, &call.target)?;
        let spec = match &callee {
            Callee::Native(method) => &method.spec,
            Callee::Conn(_, spec) => spec,
        };
        let (kind, lane) = (kind_of(spec), lane_of(spec));
        let window = match kind {
            Kind::Stream => Some(stream_window(&call.method, call.window)?),
            Kind::Unary => None,
        };
        // A relayed bulk stream: the provider gets at most RELAY_WINDOW ahead,
        // and the reader's relayed bulk streams RELAY_BUDGET in all (B7, the
        // memory of the relay).
        let relay_grant = match (&callee, window, lane) {
            (Callee::Conn(..), Some(window), Lane::Bulk) => Some(window.min(RELAY_WINDOW)),
            _ => None,
        };
        let outstanding = st.conns.get(&from).map_or(0, |c| c.relay_outstanding);
        if let Some(grant) = relay_grant
            && outstanding + grant > RELAY_BUDGET
        {
            return Err(busy(format!(
                "{} not sent: the streams this connection reads hold {outstanding} bytes in the broker",
                call.method
            )));
        }

        let timeout_ms = self.timeout_ms(call.timeout_ms, kind == Kind::Stream);
        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms.into()));
        let identity = principal.identity.to_string();
        let Some(caller) = st.conns.get_mut(&from) else {
            return Err(BusError::unavailable("gone"));
        };
        match callee {
            Callee::Native(method) => {
                let cancel = CancelToken::new();
                let generation = self.hop();
                let timer = deadline
                    .zip(timeout_ms)
                    .map(|(at, ms)| ((at, from, call.id, generation), ms));
                let ctx = CallCtx {
                    id: call.id,
                    method: call.method,
                    target: call.target,
                    from: principal.identity.clone(),
                    principal: Arc::clone(principal),
                    call_credential: (!call.credential.is_empty()).then_some(call.credential),
                    deadline: deadline.map(Instant::into_std),
                    cancel: cancel.clone(),
                    content_type: call.content_type,
                    trace_id: call.trace_id,
                };
                caller.calls.insert(
                    ctx.id,
                    Route {
                        to: To::Native {
                            generation,
                            cancel,
                            abort: None,
                        },
                        method: ctx.method.clone(),
                        timer,
                    },
                );
                if let Some(window) = window {
                    caller
                        .flows
                        .insert(ctx.id, Flow::new(generation, lane, window, window, None));
                }
                if let Some((key, _)) = timer {
                    self.arm(key);
                }
                Ok(Routed::Native(
                    method,
                    ctx,
                    call.payload,
                    generation,
                    window.unwrap_or(0),
                ))
            }
            Callee::Conn(callee, spec) => {
                let hop = self.hop();
                let timer = deadline
                    .zip(timeout_ms)
                    .map(|(at, ms)| ((at, from, call.id, hop), ms));
                caller.calls.insert(
                    call.id,
                    Route {
                        to: To::Remote { callee, hop },
                        method: call.method.clone(),
                        timer,
                    },
                );
                let forwarded_window = relay_grant.or(window);
                if let (Some(window), Some(avail)) = (window, forwarded_window) {
                    caller.flows.insert(
                        call.id,
                        Flow::new(hop, lane, window, avail, Some((callee, hop))),
                    );
                    caller.relay_outstanding += relay_grant.unwrap_or(0);
                }
                let Some(callee_conn) = st.conns.get_mut(&callee) else {
                    return Err(BusError::unavailable("gone"));
                };
                callee_conn.served.insert(
                    hop,
                    Served {
                        caller: Caller::Conn {
                            conn: from,
                            id: call.id,
                        },
                        kind,
                        lane: lane_of(&spec),
                        offered: false,
                    },
                );
                let callee_out = callee_conn.out.clone();
                let callee_identity = callee_conn.identity.clone();
                if let Some((key, _)) = timer {
                    self.arm(key);
                }
                Ok(Routed::Remote {
                    callee,
                    callee_out,
                    identity: callee_identity,
                    hop,
                    call: Call {
                        id: hop,
                        from: identity,
                        timeout_ms: timeout_ms.unwrap_or(0),
                        window: forwarded_window
                            .map_or(call.window, |window| u32::try_from(window).unwrap_or(0)),
                        ..call
                    },
                })
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
                .filter(|id| {
                    st.conns
                        .get(id)
                        .is_some_and(|conn| conn.provides.contains_key(method))
                })
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
            if !st.known(method) {
                return Err(BusError::new(
                    codes::NOT_FOUND,
                    format!("no method {method}: no participant provides it"),
                ));
            }
            return Err(BusError::unavailable(format!(
                "no provider of {method} is connected"
            )));
        }
        let spec_of = |st: &State, id: ConnId| {
            st.conns
                .get(&id)
                .and_then(|conn| conn.provides.get(method))
                .cloned()
                .unwrap_or_default()
        };
        let first = spec_of(st, candidates[0]);
        let chosen = match Select::try_from(first.select).unwrap_or(Select::RoundRobin) {
            Select::LastActive => candidates
                .iter()
                .copied()
                .max_by_key(|id| (st.conns.get(id).map_or(0, |conn| conn.activated), *id))
                .unwrap_or(candidates[0]),
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
        window: u64,
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
                    window,
                    cancel,
                    done: false,
                };

                tokio::spawn(handler(ctx, payload, out)).abort_handle()
            }
        };
        if let Some(conn) = self.lock().conns.get_mut(&from)
            && let Some(Route {
                to:
                    To::Native {
                        generation: current,
                        abort: slot,
                        ..
                    },
                ..
            }) = conn.calls.get_mut(&id)
            && *current == generation
        {
            *slot = Some(abort);
        }
    }

    /// Take the route of the native call `id` of `conn` if it is still the call of
    /// `generation` (not cancelled, timed out, nor replaced by a later call of the
    /// same id), with the caller's writer.
    fn take_native(&self, conn: ConnId, id: u64, generation: u64) -> Option<Out> {
        let mut st = self.lock();
        let caller = st.conns.get_mut(&conn)?;
        if !caller
            .calls
            .get(&id)
            .is_some_and(|route| route.is_native(generation))
        {
            return None;
        }
        let route = caller.calls.remove(&id)?;
        self.disarm(&route);
        Some(caller.out.clone())
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

    /// End the native stream `id` of `generation` with `error` (behind what it
    /// queued) and stop its handler.
    fn fail_native(&self, conn: ConnId, id: u64, generation: u64, error: BusError) {
        let (route, sends) = {
            let mut st = self.lock();
            let route = self.end_route(&mut st, conn, id, generation);
            let mut grants = Vec::new();
            if route.is_some() {
                answer_msg(
                    &mut st,
                    conn,
                    id,
                    generation,
                    error_reply(id, error),
                    &mut grants,
                );
            }
            (route, grant_sends(&st, grants))
        };
        self.send_all(sends);
        if let Some(route) = route {
            route.stop();
        }
    }

    /// Queue the chunk `packet` (`len` payload bytes) of a native stream if its
    /// credit, the reader's connection window and room in its bulk lane let it:
    /// checked, taken and queued under one lock, so nothing of a stream that is
    /// over (timed out, cancelled, its id reused) is queued, and nothing comes
    /// after its end.
    fn native_chunk(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        lane: Lane,
        packet: &Bytes,
        len: u64,
    ) -> Reserve {
        let mut st = self.lock();
        let Some(c) = st.conns.get_mut(&conn) else {
            return Reserve::Closed;
        };
        if !c
            .calls
            .get(&id)
            .is_some_and(|route| route.is_native(generation))
        {
            return Reserve::Closed;
        }
        let bulk = lane == Lane::Bulk;
        let window_open =
            !bulk || c.bulk_uncredited == 0 || c.bulk_uncredited + len <= CONNECTION_WINDOW;
        let room = !bulk || c.out.bulk_room(packet.len());
        let Some(flow) = c
            .flows
            .get_mut(&id)
            .filter(|flow| flow.unique == generation)
        else {
            return Reserve::Closed;
        };
        if len > flow.window && flow.avail < len {
            return Reserve::TooLarge(flow.window);
        }
        if flow.avail < len || !window_open || !room {
            return Reserve::Wait(Arc::clone(&c.moved));
        }
        flow.avail -= len;
        if bulk {
            flow.uncredited += len;
            c.bulk_uncredited += len;
        }
        if c.out.send_packet(packet.clone(), lane) {
            Reserve::Go
        } else {
            Reserve::Closed
        }
    }

    /// A native stream's chunk, encoded for its reader; `Err` once the stream is
    /// over (a chunk over the frame limit ends it).
    fn encode_native_chunk(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        chunk: Chunk,
    ) -> Result<(Bytes, u64), Closed> {
        let len = chunk.payload.len() as u64;
        let out = self
            .lock()
            .conns
            .get(&conn)
            .map(|c| c.out.clone())
            .ok_or(Closed)?;
        let msg = Msg::Chunk(tilr_bus::bus::Chunk {
            id,
            payload: chunk.payload,
            content_encoding: chunk.content_encoding,
        });
        match out.try_encode(msg) {
            Ok(packet) => Ok((packet, len)),
            Err(err) => {
                self.fail_native(
                    conn,
                    id,
                    generation,
                    BusError::new(codes::TOO_LARGE, format!("a chunk not sent: {err}")),
                );
                Err(Closed)
            }
        }
    }

    fn too_large(&self, conn: ConnId, id: u64, generation: u64, len: u64, window: u64) {
        self.fail_native(
            conn,
            id,
            generation,
            BusError::new(
                codes::TOO_LARGE,
                format!("a chunk of {len} bytes is larger than the stream's window of {window}"),
            ),
        );
    }

    pub(super) async fn stream_chunk(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        lane: Lane,
        chunk: Chunk,
    ) -> Result<(), Closed> {
        let (packet, len) = self.encode_native_chunk(conn, id, generation, chunk)?;
        loop {
            let moved = match self.native_chunk(conn, id, generation, lane, &packet, len) {
                Reserve::Go => return Ok(()),
                Reserve::Closed => return Err(Closed),
                Reserve::TooLarge(window) => {
                    self.too_large(conn, id, generation, len, window);
                    return Err(Closed);
                }
                Reserve::Wait(moved) => moved,
            };
            let notified = moved.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            // Credit may have come between the check and `enable`.
            match self.native_chunk(conn, id, generation, lane, &packet, len) {
                Reserve::Go => return Ok(()),
                Reserve::Closed => return Err(Closed),
                Reserve::TooLarge(window) => {
                    self.too_large(conn, id, generation, len, window);
                    return Err(Closed);
                }
                Reserve::Wait(_) => notified.await,
            }
        }
    }

    pub(super) fn try_stream_chunk(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        lane: Lane,
        chunk: Chunk,
    ) -> Result<(), TrySendError> {
        let kept = chunk.clone();
        let (packet, len) = self
            .encode_native_chunk(conn, id, generation, chunk)
            .map_err(|Closed| TrySendError::Closed)?;
        match self.native_chunk(conn, id, generation, lane, &packet, len) {
            Reserve::Go => Ok(()),
            Reserve::Wait(_) => Err(TrySendError::Full(kept)),
            Reserve::Closed => Err(TrySendError::Closed),
            Reserve::TooLarge(window) => {
                self.too_large(conn, id, generation, len, window);
                Err(TrySendError::Closed)
            }
        }
    }

    /// The end of a native stream: `End` or an error `Reply`, behind its chunks on
    /// the stream's lane (encoded before the lock is taken). Never waits for
    /// credit.
    pub(super) fn stream_end(
        &self,
        conn: ConnId,
        id: u64,
        generation: u64,
        _lane: Lane,
        result: Result<Bytes, BusError>,
    ) {
        let msg = match result {
            Ok(payload) => Msg::End(End { id, payload }),
            Err(error) => error_reply(id, error),
        };
        let out = self.lock().conns.get(&conn).map(|c| c.out.clone());
        let Some(packet) = out.and_then(|out| out.encode(msg)) else {
            return;
        };
        let sends = {
            let mut st = self.lock();
            if self.end_route(&mut st, conn, id, generation).is_none() {
                return;
            }
            let mut grants = Vec::new();
            answer(&mut st, conn, id, generation, packet, &mut grants);
            grant_sends(&st, grants)
        };
        self.send_all(sends);
    }

    /// Whether a chunk of `need` bytes of the stream could be queued now (its
    /// credit, and for a bulk stream the connection window and the bulk lane's
    /// room); `true` when the stream or the connection is gone.
    fn can_take(c: &Conn, id: u64, generation: u64, need: u64) -> bool {
        c.flows
            .get(&id)
            .filter(|flow| flow.unique == generation)
            .is_none_or(|flow| {
                flow.avail >= need
                    && (flow.lane != Lane::Bulk
                        || ((c.bulk_uncredited == 0
                            || c.bulk_uncredited + need <= CONNECTION_WINDOW)
                            && c.out.bulk_room(usize::try_from(need).unwrap_or(usize::MAX))))
            })
    }

    /// Whether the reader of a native stream lags: its connection's interactive
    /// lane has held a packet for over 250 ms, or the stream has no credit left
    /// (for a bulk stream: or the connection window is spent, or its bulk lane
    /// full).
    pub(super) fn stream_lagging(&self, conn: ConnId, id: u64, generation: u64) -> bool {
        let st = self.lock();
        let Some(c) = st.conns.get(&conn) else {
            return false;
        };
        if c.out.lagging() {
            return true;
        }
        c.flows
            .get(&id)
            .filter(|flow| flow.unique == generation)
            .is_some()
            && !Self::can_take(c, id, generation, 1)
    }

    /// Returns once the reader's interactive lane is written out and a chunk of
    /// `need` bytes could be queued (credit, and for a bulk stream the
    /// connection window and the bulk lane's room); at once when the stream or
    /// the connection is gone, or when `need` is larger than the stream's window
    /// (a send of it ends the stream `too_large`).
    pub(super) async fn stream_caught_up(&self, conn: ConnId, id: u64, generation: u64, need: u64) {
        let out = self.lock().conns.get(&conn).map(|c| c.out.clone());
        let Some(out) = out else {
            return;
        };
        out.caught_up().await;
        loop {
            let Some(moved) = self.lock().conns.get(&conn).map(|c| Arc::clone(&c.moved)) else {
                return;
            };
            let notified = moved.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let ready = {
                let st = self.lock();
                st.conns.get(&conn).is_none_or(|c| {
                    c.flows
                        .get(&id)
                        .filter(|flow| flow.unique == generation)
                        .is_some_and(|flow| need > flow.window && flow.avail < need)
                        || Self::can_take(c, id, generation, need.max(1))
                })
            };
            if ready {
                return;
            }
            notified.await;
        }
    }

    /// The bulk lane of `conn` has room again: queue what waited for it.
    pub(super) fn bulk_room(&self, conn: ConnId) {
        let sends = {
            let mut st = self.lock();
            let Some(c) = st.conns.get_mut(&conn) else {
                return;
            };
            let mut grants = Vec::new();
            pump(c, &mut grants);
            c.moved.notify_waiters();
            grant_sends(&st, grants)
        };
        self.send_all(sends);
    }

    /// Who reads what the connection `from` sends on its hop `hop`: the caller,
    /// its id and its writer, and the call's kind; `None` for a late, forged or
    /// native call.
    fn reader_of(&self, from: ConnId, hop: u64) -> Option<(ConnId, u64, Out, Kind)> {
        let st = self.lock();
        let served = st.conns.get(&from)?.served.get(&hop)?;
        let Caller::Conn { conn, id } = &served.caller else {
            return None;
        };
        let out = st.conns.get(conn)?.out.clone();
        Some((*conn, *id, out, served.kind))
    }

    /// Whether the hop `hop` of `from` is still the call `id` of `caller`.
    fn still_served(st: &State, from: ConnId, hop: u64, caller: ConnId, id: u64) -> bool {
        st.conns
            .get(&from)
            .and_then(|conn| conn.served.get(&hop))
            .is_some_and(|served| {
                matches!(served.caller, Caller::Conn { conn, id: call } if conn == caller && call == id)
            })
    }

    fn on_reply(&self, from: ConnId, reply: WireReply) {
        let hop = reply.id;
        let Some((caller, id, out, kind)) = self.reader_of(from, hop) else {
            // tilrd's own call; or a call whose caller is gone, or a late,
            // cancelled or forged reply.
            let served = self
                .lock()
                .conns
                .get_mut(&from)
                .and_then(|conn| conn.served.remove(&hop));
            if let Some(Served {
                caller: Caller::Native(tx),
                ..
            }) = served
            {
                let _ = tx.send(wire_to_result(reply));
            }
            return;
        };
        // Encoded before the lock is taken: a reply may be 16 MiB.
        let msg = if kind == Kind::Stream {
            match reply.result {
                Some(reply::Result::Error(error)) => Msg::Reply(WireReply {
                    id,
                    result: Some(reply::Result::Error(error)),
                    content_type: reply.content_type,
                }),
                _ => error_reply(
                    id,
                    BusError::bad_reply("a stream was answered with a reply instead of chunks"),
                ),
            }
        } else {
            Msg::Reply(WireReply { id, ..reply })
        };
        let Some(packet) = out.encode(msg) else {
            return;
        };
        let sends = {
            let mut st = self.lock();
            if !Self::still_served(&st, from, hop, caller, id) {
                return;
            }
            if let Some(conn) = st.conns.get_mut(&from) {
                conn.served.remove(&hop);
            }
            if self.end_route(&mut st, caller, id, hop).is_none() {
                return;
            }
            let mut grants = Vec::new();
            if kind == Kind::Stream {
                answer(&mut st, caller, id, hop, packet, &mut grants);
                grant_sends(&st, grants)
            } else {
                drop(st);
                out.send_packet(packet, Lane::Interactive);
                return;
            }
        };
        self.send_all(sends);
    }

    fn send_all(&self, sends: Vec<(Out, Msg)>) {
        for (out, msg) in sends {
            out.send(msg, Lane::Interactive);
        }
    }

    /// End the relayed stream of hop `hop` from the provider `from` (the call
    /// `id` of `caller`) with `error` behind what the broker holds of it, under
    /// the lock; returns the provider's writer, for its `Cancel`.
    fn end_relayed(
        &self,
        st: &mut State,
        from: ConnId,
        hop: u64,
        (caller, id): (ConnId, u64),
        error: impl FnOnce(&str) -> BusError,
        grants: &mut Vec<Grant>,
    ) -> Option<Out> {
        let provider = st.conns.get_mut(&from).map(|conn| {
            conn.served.remove(&hop);
            conn.out.clone()
        });
        let method = self
            .end_route(st, caller, id, hop)
            .map(|route| route.method)
            .unwrap_or_default();
        answer_msg(st, caller, id, hop, error_reply(id, error(&method)), grants);
        provider
    }

    fn on_chunk(&self, from: ConnId, chunk: tilr_bus::bus::Chunk) {
        let hop = chunk.id;
        // Who reads it, for its framing; encoded outside the lock.
        let Some((caller, id, out, Kind::Stream)) = self.reader_of(from, hop) else {
            return;
        };
        let len = chunk.payload.len() as u64;
        let encoded = out.try_encode(Msg::Chunk(tilr_bus::bus::Chunk { id, ..chunk }));
        let mut grants = Vec::new();
        let mut cancel: Option<(Out, &'static str)> = None;
        let sends = {
            let mut st = self.lock();
            if !Self::still_served(&st, from, hop, caller, id) {
                return;
            }
            let packet = match encoded {
                Ok(packet) => packet,
                Err(err) => {
                    let provider = self.end_relayed(
                        &mut st,
                        from,
                        hop,
                        (caller, id),
                        |method| {
                            BusError::new(
                                codes::TOO_LARGE,
                                format!("{method}: a chunk cannot be relayed: {err}"),
                            )
                        },
                        &mut grants,
                    );
                    cancel = provider.map(|out| (out, "a chunk that cannot be relayed"));
                    let sends = grant_sends(&st, grants);
                    drop(st);
                    if let Some((out, reason)) = cancel {
                        cancel_hop(&out, hop, reason);
                    }
                    self.send_all(sends);
                    return;
                }
            };
            let Some(c) = st.conns.get_mut(&caller) else {
                return;
            };
            let Some(flow) = c.flows.get_mut(&id).filter(|flow| flow.unique == hop) else {
                return;
            };
            let cost = chunk_cost(len);
            if len > flow.avail {
                tracing::info!(target: "tilrd::bus", connection = from, "a provider overran a stream's credit");
                let provider = self.end_relayed(
                    &mut st,
                    from,
                    hop,
                    (caller, id),
                    |method| {
                        BusError::bad_reply(format!(
                            "{method}: the provider sent more than the stream's credit"
                        ))
                    },
                    &mut grants,
                );
                cancel = provider.map(|out| (out, "credit overrun"));
            } else if flow.lane == Lane::Bulk && c.relay_outstanding - len + cost > RELAY_BUDGET {
                // Chunks far smaller than CHUNK_COST piled up for a reader
                // that does not take them.
                tracing::info!(target: "tilrd::bus", connection = from, "a provider's small chunks filled a reader's relay budget");
                let provider = self.end_relayed(
                    &mut st,
                    from,
                    hop,
                    (caller, id),
                    |method| {
                        BusError::bad_reply(format!(
                            "{method}: the reader holds too many small chunks of it ({RELAY_BUDGET} bytes counting each as at least {CHUNK_COST})"
                        ))
                    },
                    &mut grants,
                );
                cancel = provider.map(|out| (out, "too many small chunks held"));
            } else {
                flow.avail -= len;
                if flow.lane == Lane::Bulk {
                    flow.held.push_back(HeldChunk {
                        packet,
                        bytes: len,
                        cost,
                    });
                    flow.held_cost += cost;
                    c.relay_outstanding = c.relay_outstanding - len + cost;
                    if !flow.queued {
                        flow.queued = true;
                        c.ready.push_back((id, hop));
                    }
                    pump(c, &mut grants);
                } else {
                    c.out.send_packet(packet, Lane::Interactive);
                }
            }
            grant_sends(&st, grants)
        };
        if let Some((out, reason)) = cancel {
            cancel_hop(&out, hop, reason);
        }
        self.send_all(sends);
    }

    fn on_end(&self, from: ConnId, end: End) {
        let hop = end.id;
        let Some((caller, id, out, kind)) = self.reader_of(from, hop) else {
            // tilrd's own unary call answered with End.
            let served = self
                .lock()
                .conns
                .get_mut(&from)
                .and_then(|conn| conn.served.remove(&hop));
            if let Some(Served {
                caller: Caller::Native(tx),
                ..
            }) = served
            {
                let _ = tx.send(Err(BusError::bad_reply("a call was answered with End")));
            }
            return;
        };
        // Encoded before the lock is taken: its payload may be 16 MiB.
        let msg = if kind == Kind::Stream {
            Msg::End(End { id, ..end })
        } else {
            error_reply(id, BusError::bad_reply("a call was answered with End"))
        };
        let Some(packet) = out.encode(msg) else {
            return;
        };
        let sends = {
            let mut st = self.lock();
            if !Self::still_served(&st, from, hop, caller, id) {
                return;
            }
            if let Some(conn) = st.conns.get_mut(&from) {
                conn.served.remove(&hop);
            }
            if self.end_route(&mut st, caller, id, hop).is_none() {
                return;
            }
            let mut grants = Vec::new();
            answer(&mut st, caller, id, hop, packet, &mut grants);
            grant_sends(&st, grants)
        };
        self.send_all(sends);
    }

    fn on_cancel(&self, from: ConnId, cancel: Cancel) {
        let (route, forward, sends) = {
            let mut st = self.lock();
            let Some(conn) = st.conns.get_mut(&from) else {
                return;
            };
            let route = conn.calls.remove(&cancel.id);
            // The stream ends for its reader, whatever of it the broker holds.
            let unique = route
                .as_ref()
                .map(Route::unique)
                .or_else(|| conn.flows.get(&cancel.id).map(|flow| flow.unique));
            if let Some(unique) = unique {
                conn.drop_flow(cancel.id, unique);
            }
            let mut grants = Vec::new();
            pump(conn, &mut grants);
            if let Some(route) = &route {
                self.disarm(route);
            }
            let forward = match route.as_ref().map(|route| &route.to) {
                Some(To::Remote { callee, hop }) => {
                    let hop = *hop;
                    st.conns.get_mut(callee).and_then(|callee| {
                        let offered = callee.served.remove(&hop)?.offered;
                        offered.then(|| (callee.out.clone(), hop))
                    })
                }
                _ => None,
            };
            (route, forward, grant_sends(&st, grants))
        };
        if let Some(route) = route {
            route.stop();
        }
        if let Some((out, hop)) = forward {
            out.send(
                Msg::Cancel(Cancel {
                    id: hop,
                    reason: cancel.reason,
                }),
                Lane::Interactive,
            );
        }
        self.send_all(sends);
    }

    /// The reader returned credit: to its producer (relayed as is, granted by the
    /// broker for a relayed bulk stream), and to its connection window.
    fn on_credit(&self, from: ConnId, credit: Credit) {
        let bytes = u64::from(credit.bytes);
        let sends = {
            let mut st = self.lock();
            let Some(c) = st.conns.get_mut(&from) else {
                return;
            };
            let Some(flow) = c.flows.get_mut(&credit.id) else {
                return; // A stream that ended.
            };
            let mut grants = Vec::new();
            let mut forward = None;
            if flow.lane == Lane::Bulk {
                let returned = bytes.min(flow.uncredited);
                flow.uncredited -= returned;
                c.bulk_uncredited = c.bulk_uncredited.saturating_sub(returned);
            }
            if flow.relayed_bulk() {
                flow.reader_avail = flow.reader_avail.saturating_add(bytes);
                grants.extend(grant(flow, &mut c.relay_outstanding, &mut c.relay_starved));
            } else if let Some((provider, hop)) = flow.provider {
                flow.avail = flow.avail.saturating_add(bytes);
                forward = Some((provider, hop));
            } else if !flow.relayed {
                flow.avail = flow.avail.saturating_add(bytes);
            }
            pump(c, &mut grants);
            c.moved.notify_waiters();
            let mut sends = grant_sends(&st, grants);
            if let Some((provider, hop)) = forward
                && let Some(provider) = st.conns.get(&provider)
            {
                sends.push((
                    provider.out.clone(),
                    Msg::Credit(Credit {
                        id: hop,
                        bytes: credit.bytes,
                    }),
                ));
            }
            sends
        };
        self.send_all(sends);
    }

    /// A call's deadline passed: the caller gets `timeout` (a stream's behind its
    /// chunks), the callee `Cancel`.
    fn expire(&self, key: TimerKey) {
        let (_, conn, id, unique) = key;
        let (route, forward, sends) = {
            let mut st = self.lock();
            let Some(c) = st.conns.get_mut(&conn) else {
                return;
            };
            let current = c.calls.get(&id).is_some_and(|route| {
                route.unique() == unique && route.timer.map(|t| t.0) == Some(key)
            });
            if !current {
                return;
            }
            let Some(route) = c.calls.remove(&id) else {
                return;
            };
            let ms = route.timer.map_or(0, |timer| timer.1);
            let forward = match &route.to {
                To::Remote { callee, hop } => {
                    let hop = *hop;
                    st.conns.get_mut(callee).and_then(|callee| {
                        let offered = callee.served.remove(&hop)?.offered;
                        offered.then(|| (callee.out.clone(), hop))
                    })
                }
                To::Native { .. } => None,
            };
            let mut grants = Vec::new();
            answer_msg(
                &mut st,
                conn,
                id,
                unique,
                error_reply(id, timed_out(&route.method, ms)),
                &mut grants,
            );
            (route, forward, grant_sends(&st, grants))
        };
        route.stop();
        if let Some((out, hop)) = forward {
            out.send(
                Msg::Cancel(Cancel {
                    id: hop,
                    reason: "timeout".into(),
                }),
                Lane::Interactive,
            );
        }
        self.send_all(sends);
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

    /// Deliver an event to every connection subscribed to it now; one whose lane
    /// lags does not get it (B6), and that is counted.
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
        let mut dropped = 0;
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
            let Some(packet) = out.encode(Msg::Event(WireEvent {
                subject: subject.clone(),
                payload: payload.clone(),
                content_type: content_type.clone(),
                from: from.clone(),
                scope: scope.clone(),
            })) else {
                continue;
            };
            if out.offer(packet, true) == Offer::Full {
                dropped += 1;
            }
        }
        if dropped > 0 {
            events::count_dropped(&subject, dropped);
        }
    }

    // -----------------------------------------------------------------------
    // Calls by tilrd's core
    // -----------------------------------------------------------------------

    /// Call a method of another participant as `from` (tilrd's core: `host.open`,
    /// `ssh_agent.*`). A `target` in `options` names the participant. The deadline
    /// is `options.timeout` as given (not capped by `call_timeout_ms`, which a
    /// call over the wire is); without one, `call_timeout_ms` (none if it is 0).
    /// At the deadline the callee gets `Cancel`.
    pub async fn call(
        &self,
        from: Identity,
        method: &str,
        payload: impl Into<Bytes>,
        options: CallOptions,
    ) -> Result<Reply, BusError> {
        let target = options.target.clone().unwrap_or_default();
        let timeout_ms = match options.timeout {
            Some(timeout) => Some(u32::try_from(timeout.as_millis().max(1)).unwrap_or(u32::MAX)),
            None => self.timeout_ms(0, false),
        };
        let (tx, rx) = oneshot::channel();
        let hop = self.hop();
        let (out, identity, call) = {
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
            let Some(conn) = st.conns.get_mut(&callee) else {
                return Err(BusError::unavailable(format!(
                    "{method}: the callee is gone"
                )));
            };
            conn.served.insert(
                hop,
                Served {
                    caller: Caller::Native(tx),
                    kind: kind_of(&spec),
                    lane: lane_of(&spec),
                    offered: false,
                },
            );
            (
                conn.out.clone(),
                conn.identity.clone(),
                Call {
                    id: hop,
                    target,
                    method: method.to_owned(),
                    payload: payload.into(),
                    from: from.to_string(),
                    trace_id: options.trace_id.unwrap_or_default(),
                    timeout_ms: timeout_ms.unwrap_or(0),
                    content_type: options.content_type.unwrap_or_default(),
                    credential: Bytes::new(),
                    window: 0,
                },
            )
        };
        let callee_id = out.id();
        let forget = || {
            if let Some(conn) = self.lock().conns.get_mut(&callee_id) {
                conn.served.remove(&hop);
            }
        };
        let offered = match out.encode(Msg::Call(call)) {
            Some(packet) => out.offer(packet, false),
            None => Offer::Gone,
        };
        match offered {
            Offer::Queued => {
                let ended = {
                    let mut st = self.lock();
                    match st
                        .conns
                        .get_mut(&callee_id)
                        .and_then(|conn| conn.served.get_mut(&hop))
                    {
                        Some(served) => {
                            served.offered = true;
                            false
                        }
                        None => true,
                    }
                };
                if ended {
                    cancel_hop(&out, hop, "the call ended before it was sent");
                }
            }

            Offer::Full => {
                forget();
                return Err(busy(format!(
                    "{method} not sent: {identity} is busy, its queue is full"
                )));
            }
            Offer::Gone => {
                forget();
                return Err(BusError::unavailable(format!("{identity} disconnected")));
            }
        }
        let answer = async {
            rx.await
                .unwrap_or_else(|_| Err(BusError::unavailable("the callee is gone")))
        };
        match timeout_ms {
            None => answer.await,
            Some(ms) => {
                match tokio::time::timeout(Duration::from_millis(ms.into()), answer).await {
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
                        Err(timed_out(method, ms))
                    }
                }
            }
        }
    }
}

/// What became of a native producer's chunk.
enum Reserve {
    /// Credit and window taken, and it is queued.
    Go,
    /// Wait until this is notified, then ask again.
    Wait(Arc<Notify>),
    /// Larger than the stream's window (and its credit): it can never go.
    TooLarge(u64),
    Closed,
}

/// Tell a provider its stream of `hop` is over.
fn cancel_hop(out: &Out, hop: u64, reason: &str) {
    out.send(
        Msg::Cancel(Cancel {
            id: hop,
            reason: reason.into(),
        }),
        Lane::Interactive,
    );
}

enum Routed {
    Remote {
        callee: ConnId,
        callee_out: Out,
        /// The callee's identity, for the errors.
        identity: String,
        hop: u64,
        call: Call,
    },
    /// The method, the call, its payload, its generation and (a stream) its
    /// window.
    Native(NativeMethod, CallCtx, Bytes, u64, u64),
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
            NativeMethod::stream("test.latest", Lane::Bulk, latest),
        ]);
        broker
    }

    /// Times `test.latest` asked whether it may send again.
    static LATEST_WAKES: AtomicU64 = AtomicU64::new(0);

    /// A latest-wins producer: `<total> <chunk>` bytes in chunks of `chunk`
    /// with `try_send`, waiting with `caught_up_for` when it is full; ends with
    /// the window it was given.
    async fn latest(_: CallCtx, payload: Bytes, out: StreamOut) {
        let text = String::from_utf8_lossy(&payload).into_owned();
        let mut parts = text.split(' ').map(|part| part.parse::<usize>().unwrap());
        let (total, chunk) = (parts.next().unwrap(), parts.next().unwrap());
        let mut sent = 0;
        while sent < total {
            match out.try_send(Chunk::new(vec![b'l'; chunk])) {
                Ok(()) => sent += chunk,
                Err(TrySendError::Full(_)) => {
                    out.caught_up_for(chunk as u64).await;
                    LATEST_WAKES.fetch_add(1, Ordering::Relaxed);
                    // Were `caught_up_for` to return at once, this loop would
                    // hold the test's thread: let the test see it spin.
                    tokio::task::yield_now().await;
                }
                Err(TrySendError::Closed) => return,
            }
        }
        let window = out.window().to_string();
        out.end(Bytes::from(window));
    }

    /// How long a test waits for what it awaits before it fails.
    const WAIT: Duration = Duration::from_secs(10);

    /// `future` within [`WAIT`]: a regression fails the test instead of hanging it.
    async fn within<F: std::future::Future>(future: F) -> F::Output {
        tokio::time::timeout(WAIT, future)
            .await
            .expect("not done in time")
    }

    /// An instance on a stream socket, greeted, with the right to call `test.*`.
    struct Peer0 {
        stream: UnixStream,
        buf: BytesMut,
    }

    impl Peer0 {
        async fn attach(broker: &BrokerHandle) -> Self {
            Self::attach_as(broker, "t-1", Vec::new()).await.0
        }

        /// The instance `name`, providing `provides`, and its connection id.
        async fn attach_as(
            broker: &BrokerHandle,
            name: &str,
            provides: Vec<MethodSpec>,
        ) -> (Self, ConnId) {
            let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
            let attached = broker.attach_stream(
                ours.into(),
                Seat {
                    identity: Identity::Instance(name.into()),
                    rights: Some(Arc::new(Rights {
                        call: vec!["test.*".into()],
                        provide: vec!["test.*".into()],
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
                client_name: name.into(),
                provides,
                ..Hello::default()
            }))
            .await;
            assert!(matches!(peer.recv().await, Msg::HelloAck(_)));
            (peer, attached.connection_id)
        }

        async fn call_with(&mut self, id: u64, method: &str, payload: &str, window: u32) {
            self.send(Msg::Call(Call {
                id,
                method: method.into(),
                target: "tilrd".into(),
                payload: Bytes::copy_from_slice(payload.as_bytes()),
                window,
                ..Call::default()
            }))
            .await;
        }

        /// The next envelope but `Chunk`s, and the chunk bytes before it.
        async fn chunks_then(&mut self) -> (usize, Msg) {
            within(async {
                let mut bytes = 0;
                loop {
                    match self.recv().await {
                        Msg::Chunk(chunk) => bytes += chunk.payload.len(),
                        other => return (bytes, other),
                    }
                }
            })
            .await
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

        async fn credit(&mut self, id: u64, bytes: usize) {
            self.send(Msg::Credit(Credit {
                id,
                bytes: bytes as u32,
            }))
            .await;
        }

        /// The next envelope within `wait`, if one comes.
        async fn recv_within(&mut self, wait: Duration) -> Option<Msg> {
            tokio::time::timeout(wait, self.recv()).await.ok()
        }

        async fn recv(&mut self) -> Msg {
            tokio::time::timeout(WAIT, async {
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
                    // 2 MiB in all: more than the window, so the reader credits.
                    peer.credit(1, CHUNK).await;
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
    async fn a_native_stream_sends_no_more_than_its_credit() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        // A window of two chunks.
        peer.send(Msg::Call(Call {
            id: 1,
            method: "test.count".into(),
            target: "tilrd".into(),
            payload: Bytes::from_static(b"8"),
            window: 2 * CHUNK as u32,
            ..Call::default()
        }))
        .await;
        for _ in 0..2 {
            assert!(matches!(peer.recv().await, Msg::Chunk(chunk) if chunk.id == 1));
        }
        assert!(
            peer.recv_within(Duration::from_millis(200)).await.is_none(),
            "a third chunk without credit"
        );
        peer.credit(1, CHUNK).await;
        assert!(matches!(peer.recv().await, Msg::Chunk(chunk) if chunk.id == 1));
        assert!(peer.recv_within(Duration::from_millis(200)).await.is_none());
        // A window smaller than a chunk: the stream ends too_large.
        peer.send(Msg::Call(Call {
            id: 2,
            method: "test.count".into(),
            target: "tilrd".into(),
            payload: Bytes::from_static(b"1"),
            window: 1000,
            ..Call::default()
        }))
        .await;
        match peer.recv().await {
            Msg::Reply(WireReply {
                id: 2,
                result: Some(reply::Result::Error(error)),
                ..
            }) => assert_eq!(error.code, codes::TOO_LARGE, "{error:?}"),
            other => panic!("expected too_large, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_call_outside_its_rights_is_denied_naming_the_method() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        peer.call(3, "other.thing", "").await;
        match peer.recv().await {
            Msg::Reply(WireReply {
                id: 3,
                result: Some(reply::Result::Error(error)),
                ..
            }) => {
                assert_eq!(error.code, codes::DENIED);
                assert!(error.message.contains("other.thing"), "{error:?}");
            }
            other => panic!("expected denied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_native_call_past_its_deadline_is_answered_timeout_and_cancelled() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        peer.send(Msg::Call(Call {
            id: 4,
            method: "test.slow".into(),
            target: "tilrd".into(),
            timeout_ms: 50,
            ..Call::default()
        }))
        .await;
        match peer.recv().await {
            Msg::Reply(WireReply {
                id: 4,
                result: Some(reply::Result::Error(error)),
                ..
            }) => {
                assert_eq!(error.code, codes::TIMEOUT);
                assert!(error.message.contains("test.slow"), "{error:?}");
            }
            other => panic!("expected timeout, got {other:?}"),
        }
        // The handler was stopped: its answer at 300 ms never comes.
        assert!(peer.recv_within(Duration::from_millis(400)).await.is_none());
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

    fn error_code(msg: &Msg) -> Option<(u64, String, String)> {
        match msg {
            Msg::Reply(WireReply {
                id,
                result: Some(reply::Result::Error(error)),
                ..
            }) => Some((*id, error.code.clone(), error.message.clone())),
            _ => None,
        }
    }

    #[tokio::test]
    async fn the_end_of_a_native_stream_needs_no_credit() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        // A window of exactly its two chunks, never credited: End still comes.
        peer.call_with(1, "test.count", "2", 2 * CHUNK as u32).await;
        let (bytes, last) = peer.chunks_then().await;
        assert_eq!(bytes, 2 * CHUNK);
        assert!(
            matches!(&last, Msg::End(end) if &end.payload[..] == b"done"),
            "{last:?}"
        );
    }

    #[tokio::test]
    async fn a_native_bulk_stream_waits_for_the_connection_window() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        // 2 MiB on a window of 4 MiB, not credited: the connection window's 1 MiB.
        peer.call_with(1, "test.count", "64", 4 << 20).await;
        let mut got = 0;
        while got < CONNECTION_WINDOW as usize {
            match peer.recv().await {
                Msg::Chunk(chunk) => got += chunk.payload.len(),
                other => panic!("{other:?}"),
            }
        }
        assert!(
            peer.recv_within(Duration::from_millis(200)).await.is_none(),
            "more than the connection window"
        );
        peer.credit(1, got).await;
        let (more, last) = peer.chunks_then().await;
        assert_eq!(got + more, 64 * CHUNK);
        assert!(matches!(last, Msg::End(_)), "{last:?}");
    }

    #[tokio::test]
    async fn a_chunk_larger_than_the_window_ends_a_latest_wins_stream() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        // try_send of a chunk that can never go is Closed, not Full: the
        // producer does not loop on caught_up.
        peer.call_with(1, "test.latest", "2000 2000", 1000).await;
        let (bytes, last) = peer.chunks_then().await;
        assert_eq!(bytes, 0);
        let (id, code, message) = error_code(&last).expect("an error");
        assert_eq!((id, code.as_str()), (1, codes::TOO_LARGE), "{message}");
    }

    #[tokio::test]
    async fn a_latest_wins_producer_waits_for_credit_for_its_chunk() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        peer.call_with(1, "test.latest", "5000 1000", 4000).await;
        let mut got = 0;
        while got < 4000 {
            match peer.recv().await {
                Msg::Chunk(chunk) => got += chunk.payload.len(),
                other => panic!("{other:?}"),
            }
        }
        // Credit for half a chunk: it does not wake to find it cannot send.
        let wakes = LATEST_WAKES.load(Ordering::Relaxed);
        peer.credit(1, 500).await;
        assert!(peer.recv_within(Duration::from_millis(200)).await.is_none());
        assert_eq!(
            LATEST_WAKES.load(Ordering::Relaxed),
            wakes,
            "woke without room"
        );
        peer.credit(1, 500).await;
        let (bytes, last) = peer.chunks_then().await;
        assert_eq!(bytes, 1000);
        // StreamOut::window is the reader's window.
        assert!(
            matches!(&last, Msg::End(end) if &end.payload[..] == b"4000"),
            "{last:?}"
        );
    }

    #[tokio::test]
    async fn caught_up_waits_for_the_connection_window() {
        let broker = broker();
        let mut peer = Peer0::attach(&broker).await;
        // Credit enough (4 MiB) but a connection window of 1 MiB, not credited.
        peer.call_with(1, "test.latest", "2097152 65536", 4 << 20)
            .await;
        let mut got = 0;
        while got < CONNECTION_WINDOW as usize {
            match peer.recv().await {
                Msg::Chunk(chunk) => got += chunk.payload.len(),
                other => panic!("{other:?}"),
            }
        }
        let wakes = LATEST_WAKES.load(Ordering::Relaxed);
        assert!(peer.recv_within(Duration::from_millis(200)).await.is_none());
        assert!(
            LATEST_WAKES.load(Ordering::Relaxed) <= wakes + 1,
            "a latest-wins producer spins on a spent connection window"
        );
        peer.credit(1, got).await;
        let (more, last) = peer.chunks_then().await;
        assert_eq!(got + more, 2 << 20);
        assert!(matches!(last, Msg::End(_)), "{last:?}");
    }

    #[tokio::test]
    async fn a_draining_provider_is_unavailable() {
        let broker = broker();
        let (mut provider, provider_id) =
            Peer0::attach_as(&broker, "p-1", vec![MethodSpec::unary("test.drained")]).await;
        let mut caller = Peer0::attach_as(&broker, "c-1", Vec::new()).await.0;
        let by_method = |id| {
            Msg::Call(Call {
                id,
                method: "test.drained".into(),
                ..Call::default()
            })
        };
        broker.set_draining(provider_id, true);
        caller.send(by_method(1)).await;
        let (id, code, message) = error_code(&caller.recv().await).expect("an error");
        assert_eq!((id, code.as_str()), (1, codes::UNAVAILABLE));
        assert_eq!(message, "no provider of test.drained is connected");
        broker.set_draining(provider_id, false);
        caller.send(by_method(2)).await;
        assert!(matches!(provider.recv().await, Msg::Call(call) if call.method == "test.drained"));
    }

    #[tokio::test]
    async fn a_native_callers_own_deadline_is_kept() {
        let broker = broker();
        broker.set_call_timeout(Duration::from_millis(100));
        let (mut provider, _) =
            Peer0::attach_as(&broker, "p-1", vec![MethodSpec::unary("test.wait")]).await;
        // Its own deadline, over call_timeout_ms: as given.
        let call = {
            let broker = broker.clone();
            tokio::spawn(async move {
                broker
                    .call(
                        Identity::Tilrd,
                        "test.wait",
                        Bytes::new(),
                        CallOptions::default().timeout(Duration::from_secs(5)),
                    )
                    .await
            })
        };
        let Msg::Call(forwarded) = provider.recv().await else {
            panic!("expected the call");
        };
        assert_eq!(forwarded.timeout_ms, 5000);
        provider
            .send(Msg::Reply(WireReply {
                id: forwarded.id,
                result: Some(reply::Result::Payload(Bytes::from_static(b"late"))),
                content_type: String::new(),
            }))
            .await;
        assert_eq!(&within(call).await.unwrap().unwrap().payload[..], b"late");
        // None given: call_timeout_ms, then Cancel.
        let call = {
            let broker = broker.clone();
            tokio::spawn(async move {
                broker
                    .call(
                        Identity::Tilrd,
                        "test.wait",
                        Bytes::new(),
                        CallOptions::default(),
                    )
                    .await
            })
        };
        let Msg::Call(forwarded) = provider.recv().await else {
            panic!("expected the call");
        };
        assert_eq!(forwarded.timeout_ms, 100);
        assert_eq!(
            within(call).await.unwrap().unwrap_err().code,
            codes::TIMEOUT
        );
        assert!(matches!(provider.recv().await, Msg::Cancel(cancel) if cancel.id == forwarded.id));
    }

    #[tokio::test]
    async fn the_broker_sends_a_credential_update_and_tells_when_a_connection_ends() {
        let broker = broker();
        let (mut peer, id) = Peer0::attach_as(&broker, "p-1", Vec::new()).await;
        assert!(broker.send_credential_update(id, Bytes::from_static(b"renewed")));
        assert!(matches!(
            peer.recv().await,
            Msg::CredentialUpdate(update) if &update.credential[..] == b"renewed"
        ));
        let mut state = broker.watch_connection(id).expect("connected");
        drop(peer);
        within(state.wait_for(|state| matches!(state, ConnState::Ended(_))))
            .await
            .unwrap();
        assert!(broker.watch_connection(id).is_none());
        assert!(!broker.send_credential_update(id, Bytes::new()));
    }
}
