//! `pane.*` (T2, T4, T5, T9): create, list, close, resize, input, applied,
//! input_finalize, output, read, with `PaneInfo.created_by` stamped; the pane
//! events `pane.created`, `pane.changed`, `pane.title`, `pane.bell`,
//! `pane.command_finished`, `pane.exited`, each with scope `pane:<id>`.
//!
//! Owner: `bus-core-panes`.
//!
//! Every method runs on the supervisor's existing paths through the handle: a pane
//! is started by `Supervisor::create_pane`, input goes to the pane process as the
//! `tilr.v1` packet it already reads, frames come from `on_pane_output`. What the
//! bus needs besides lives in a slot ([`PanesState`]), changed only on the
//! supervisor's task. The supervisor calls this module at five places, marked
//! "Hook of `bus-core-panes`": the pane list push ([`panes_changed`]), the input
//! acknowledgements ([`input_ack`]), a pane that failed to start
//! ([`create_failed`]), the answer to a text request ([`text_reply`]); and the
//! observer ([`BusObserver`]) gets the frames, the activity and the exits.
//!
//! **Input.** Every `pane.input` and `pane.resize` is sent to the pane process as
//! delivery input, so the pane process says when it was applied (the pty took
//! its last byte, the scroll, clear or resize took effect): numbered input
//! (`input_seq` > 0) under the identity of the caller's `client_id`, applied at
//! most once across reconnects as `tilr.v1`'s; other input under an identity of
//! its connection (`bus:<connection id>`) numbered by the core, forgotten when the
//! connection ends. The reply is that acknowledgement.
//!
//! **Output** (T4). Each `pane.output` stream has a task that owns its
//! [`StreamOut`]; the observer hands it each frame of the pane (a clone of the
//! pane process's packet) through a short queue, without a lock and without
//! waiting. The task skips frames while the consumer lags (its interactive lane
//! held a packet over 250 ms, its stream credit is spent, or its queue here was
//! full), asks the pane for a full frame once it caught up and sends that one with
//! the consumer's latest echoed seq, as `tilr.v1` does for a subscriber. A frame
//! goes out as `PaneOutput{frame}`: the pane process's encoding of the `Frame`,
//! with only the tag of the wrapping message changed (the `tilr.v1` packet holds
//! `ServerMessage{frame}`); the transport compresses it for the connection.
//! A pane's last frame is full: a consumer still behind when the program ended
//! gets it before `End`.
//!
//! **Cleanup.** The broker aborts a handler when its caller cancels: what a
//! handler leaves on the supervisor (a watcher, a text request, a wait) is
//! dropped by a guard made before the supervisor's part runs.
//!
//! **Reload.** Panes cross as today; what this module keeps does not
//! (`created_by` is `owner` and `profile` empty afterwards until `bus-reload`
//! carries them; the pane keeps running what its profile said).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use prost::Message;
use tilr_bus::bus::Lane;
use tilr_bus::methods;
use tilr_bus::pane;
use tilr_bus::{BusError, Reply, codes};
use tilr_proto::{ClientMessage, ServerMessage, client_message::Msg, server_message};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::time::Instant;

use super::json::{self, Json};
use super::{BusObserver, Core, CoreCmd, PaneEvent, Registry};
use crate::bus::authz::{AuthzRequest, ForCall};
use crate::bus::broker::ConnState;
use crate::bus::identity::{CallCtx, Identity, Principal, Transport};
use crate::bus::streams::{Chunk, NativeMethod, StreamOut, TrySendError};
use crate::link::{self, FrameHead};
use crate::supervisor::{HELLO_PROBE_TIMEOUT, PaneId, Supervisor, ask_full_frame};
use crate::time::now_ms;

/// `pane.title {pane_id, title}`; the payload is the pane's `PaneInfo`, which
/// holds both (the schema has no message of its own for it).
pub const PANE_TITLE: &str = "pane.title";

/// Frames queued for one `pane.output` task; a full queue counts as lag.
const FEED: usize = 16;
/// `pane.read`: rows of history at most, and the longest wait (T9).
const MAX_HISTORY: u32 = 10_000;
const MAX_WAIT: Duration = Duration::from_secs(25);
/// `pane.read` answers this long before the call's deadline.
const DEADLINE_MARGIN: Duration = Duration::from_millis(100);
/// How long a pane process may take to answer a text request of a call without
/// a deadline: one that does not (stuck, or an old binary after a failed
/// self-reload) gets `unavailable`.
const TEXT_WAIT: Duration = Duration::from_secs(5);
/// The `tilr.v1` client id a pane made by the bus is started for: no connection
/// has it, so the `tilr.v1` answers to it go nowhere.
const NO_CLIENT: u64 = u64::MAX;
/// `pane.applied` probes count from here, above every `tilr.v1` client id (a
/// `tilr.v1` probe is its client id plus one).
const PROBE_BASE: u64 = 1 << 63;
/// A chunk's payload at most: `max_frame` less room for the envelope.
const MAX_CHUNK: usize = tilr_bus::codec::MAX_FRAME - 1024;
/// The tag of field 5 (`frame`) of `tilr.v1`'s `ServerMessage`, and of field 1
/// (`frame`) of `PaneOutput`, both length-delimited.
const SERVER_MESSAGE_FRAME_TAG: u8 = 5 << 3 | 2;
const PANE_OUTPUT_FRAME_TAG: u8 = 1 << 3 | 2;

/// What this module keeps on the supervisor (`CoreState::slot`).
#[derive(Default)]
pub struct PanesState {
    core: Option<Core>,
    /// `PaneInfo.created_by` of the panes made on the bus; `owner` otherwise.
    creators: HashMap<PaneId, String>,
    /// `pane.create` calls waiting for their pane's program to run.
    creating: HashMap<PaneId, oneshot::Sender<Result<pane::PaneInfo, BusError>>>,
    next_create: u64,
    /// The open `pane.output` streams, by pane.
    watchers: HashMap<PaneId, Vec<Watcher>>,
    /// The `PaneInfo` last told, `used_at_ms` left out: what `pane.changed` compares.
    listed: HashMap<PaneId, pane::PaneInfo>,
    /// Input waiting for its acknowledgement, by delivery identity and pane.
    acks: HashMap<(String, PaneId), Vec<AckWait>>,
    /// The panes each numbered identity sent input to: what `pane.applied`
    /// probes.
    delivered: HashMap<String, HashSet<PaneId>>,
    /// The connections that sent input or asked for their counters.
    sessions: HashMap<u64, Session>,
    probes: HashMap<u64, Probe>,
    next_probe: u64,
    closing: HashMap<PaneId, Vec<oneshot::Sender<Result<pane::CloseReply, BusError>>>>,
    /// Text requests sent to pane processes, by request id.
    reads: HashMap<u64, ReadWait>,
    /// Panes `pane.read` waits on for a change, with the calls that wait.
    changes: HashMap<PaneId, (watch::Sender<Change>, HashSet<u64>)>,
}

/// A text request of a `pane.read` call.
struct ReadWait {
    pane: PaneId,
    /// The call's token: its guard drops what is left of it.
    call: u64,
    reply: oneshot::Sender<Result<pane::ReadReply, BusError>>,
}

/// Tokens of `pane.output` streams, `pane.read` calls and their text requests,
/// taken on the handler's side, so the guard that drops what a handler left on
/// the supervisor exists before the supervisor runs its part.
fn next_token() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One `pane.output` stream, as the supervisor's side holds it.
struct Watcher {
    token: u64,
    viewer: pane::Viewer,
    feed: mpsc::Sender<FrameHeadPacket>,
    shared: Arc<WatchShared>,
    /// The consumer's connection: its seqs are `connection_id << 32` and up.
    conn: u64,
}

/// A frame of the pane as the observer handed it on: its number among the
/// frames offered to this stream, its head, the pane process's packet.
type FrameHeadPacket = (u64, FrameHead, Bytes);

/// A full frame that did not get out, kept for the end of the stream.
type Unsent = (u64, FrameHead, Bytes);

/// Between the observer and a stream's task: atomics on the hot path; a lock
/// only when a full frame does not fit the queue.
#[derive(Default)]
struct WatchShared {
    /// Frames offered so far: numbers them.
    offered: AtomicU64,
    /// A frame did not fit the queue: the screen is behind, a full frame is due.
    missed: AtomicBool,
    /// The number of the latest frame that did not fit the queue: a full frame
    /// queued before it is older than what the consumer missed, so it does not
    /// end the lag.
    missed_upto: AtomicU64,
    /// The latest of the consumer's own seqs a missed frame echoed.
    missed_seq: AtomicU64,
    /// The latest full frame that did not fit the queue: the pane's last frame
    /// is full, so a consumer behind at the exit still gets the last screen.
    missed_full: std::sync::Mutex<Option<Unsent>>,
    /// Set when the pane's program ended.
    exit: OnceLock<i32>,
    wake: Notify,
}

impl Watcher {
    /// Hand a frame to the stream's task; never waits.
    fn offer(&self, head: FrameHead, packet: &Bytes) {
        let n = self.shared.offered.fetch_add(1, Ordering::Relaxed) + 1;
        match self.feed.try_send((n, head, packet.clone())) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                if own_seq(head.last_input_seq, self.conn) != 0 {
                    self.shared
                        .missed_seq
                        .fetch_max(head.last_input_seq, Ordering::Relaxed);
                }
                if head.full
                    && let Ok(mut kept) = self.shared.missed_full.lock()
                {
                    *kept = Some((n, head, packet.clone()));
                }
                self.shared.missed_upto.fetch_max(n, Ordering::AcqRel);
                self.shared.missed.store(true, Ordering::Release);
                self.shared.wake.notify_one();
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}

/// What `pane.read` waits for.
#[derive(Clone, Copy, Debug, Default)]
struct Change {
    frames: u64,
    exit: Option<i32>,
}

struct AckWait {
    /// The connection of the call.
    conn: u64,
    /// The delivery seq the pane process acknowledges.
    seq: u64,
    /// `InputAck.seq` of the reply: the caller's `input_seq`, 0 without one.
    input_seq: u64,
    reply: oneshot::Sender<Result<pane::InputAck, BusError>>,
}

/// A connection's input (T5).
struct Session {
    /// The delivery identity of its numbered input; empty without a `client_id`
    /// or once finalized.
    numbered: String,
    /// The identity of its input without `input_seq`, numbered by the core.
    plain: String,
    next_plain: u64,
}

struct Probe {
    identity: String,
    waiting: HashSet<PaneId>,
    applied: Vec<pane::InputAck>,
    reply: Option<oneshot::Sender<Result<pane::AppliedReply, BusError>>>,
}

/// Who calls, as the handlers need it on the supervisor's task.
#[derive(Clone)]
struct Caller {
    principal: Arc<Principal>,
    credential: Option<Bytes>,
    from: Identity,
    /// The call's method and target: the pane checks are made for that call.
    method: String,
    target: String,
}

impl Caller {
    fn of(ctx: &CallCtx) -> Self {
        Self {
            principal: Arc::clone(&ctx.principal),
            credential: ctx.call_credential.clone(),
            from: ctx.from.clone(),
            method: ctx.method.clone(),
            target: ctx.target.clone(),
        }
    }

    /// The call a pane check is made for.
    fn call(&self) -> ForCall<'_> {
        ForCall {
            method: &self.method,
            target: &self.target,
        }
    }

    fn conn(&self) -> u64 {
        self.principal.connection_id
    }
}

/// What a handler on the supervisor's task gives back: the answer, or a promise
/// of it (once the program runs, the input is applied, the program ended).
enum Answer<R> {
    Now(Result<R, BusError>),
    Later(oneshot::Receiver<Result<R, BusError>>),
}

fn state(s: &mut Supervisor) -> &mut PanesState {
    s.bus.slot::<PanesState>()
}

/// `no such pane: <name as sent>` (T5).
pub fn no_such_pane(named: &str) -> BusError {
    BusError::new(codes::NOT_FOUND, format!("no such pane: {named}"))
}

/// The pane `named` names, if it runs.
pub fn find_pane(s: &Supervisor, named: &str) -> Result<PaneId, BusError> {
    named
        .parse::<PaneId>()
        .ok()
        .filter(|id| s.panes.contains_key(id))
        .ok_or_else(|| no_such_pane(named))
}

/// `PaneInfo.created_by` of a pane: the identity that created it on the bus,
/// `owner` for a pane of `tilr.v1` or one from before a reload.
pub fn creator(s: &Supervisor, pane_id: PaneId) -> String {
    s.bus
        .slot_ref::<PanesState>()
        .and_then(|st| st.creators.get(&pane_id).cloned())
        .unwrap_or_else(|| Identity::Owner.to_string())
}

fn authorize(s: &Supervisor, caller: &Caller, request: &AuthzRequest<'_>) -> Result<(), BusError> {
    // No broker, no authorizer: nothing is allowed (fail closed).
    let Some(broker) = s.bus.broker() else {
        return Err(BusError::unavailable("the bus is not running"));
    };
    broker
        .authorizer()
        .authorize(&caller.principal, caller.credential.as_deref(), request)
        .map_err(BusError::from)
}

/// Whether the caller may access the pane (T7: `AuthzRequest::PaneAccess` with
/// its creator; the policy is the authorizer's).
fn may_access(s: &Supervisor, caller: &Caller, pane_id: PaneId) -> Result<(), BusError> {
    // The authorizer of this build refuses no `PaneAccess` a call can reach (its
    // policy allows every pane check until `bus-pane-rights`): a test stands in for
    // one that does.
    #[cfg(feature = "failpoints")]
    if crate::fail::point("bus-pane-access").is_err() {
        return Err(crate::bus::authz::Denied::new("failpoint bus-pane-access").into());
    }
    let id = pane_id.to_string();
    let creator = creator(s, pane_id);
    authorize(
        s,
        caller,
        &AuthzRequest::PaneAccess {
            pane_id: &id,
            creator: &creator,
            call: caller.call(),
        },
    )
}

/// The pane `named` names if it runs and the caller may access it. A pane the
/// caller may not access is `no such pane` as well, so a refusal does not tell
/// that it exists.
fn accessible_pane(s: &Supervisor, caller: &Caller, named: &str) -> Result<PaneId, BusError> {
    let pane_id = find_pane(s, named)?;
    match may_access(s, caller, pane_id) {
        Ok(()) => Ok(pane_id),
        Err(err) if err.code == codes::DENIED => Err(no_such_pane(named)),
        Err(err) => Err(err),
    }
}

/// The pane's `PaneInfo` as the bus gives it: `tilr.v1` subscribers and
/// `pane.output` consumers both count as viewers.
pub fn pane_info(s: &Supervisor, pane_id: PaneId) -> Option<pane::PaneInfo> {
    let p = s.panes.get(&pane_id)?;
    let mut viewed_by: Vec<pane::Viewer> = p
        .subscribers
        .iter()
        .filter_map(|id| s.clients.get(id))
        .map(|client| pane::Viewer {
            client_id: client.client_id.clone(),
            client_name: client.name.clone(),
            address: client.addr.to_string(),
            identity: Identity::Owner.to_string(),
        })
        .collect();
    let mut viewers = p.subscribers.len();
    if let Some(watchers) = s
        .bus
        .slot_ref::<PanesState>()
        .and_then(|st| st.watchers.get(&pane_id))
    {
        viewers += watchers.len();
        viewed_by.extend(watchers.iter().map(|w| w.viewer.clone()));
    }
    viewed_by.sort_by(|a, b| {
        (&a.identity, &a.client_name, &a.client_id, &a.address).cmp(&(
            &b.identity,
            &b.client_name,
            &b.client_id,
            &b.address,
        ))
    });
    Some(pane::PaneInfo {
        pane_id: pane_id.to_string(),
        cols: u32::from(p.cols),
        rows: u32::from(p.rows),
        command: p.command.clone(),
        title: p.title.clone(),
        cwd: p.cwd.clone(),
        viewers: u32::try_from(viewers).unwrap_or(u32::MAX),
        created_at_ms: p.created_at_ms,
        pid: p.pid,
        viewed_by,
        used_at_ms: p.used_at_ms,
        created_by: creator(s, pane_id),
        sandboxed: super::profiles::of(s, pane_id).is_some_and(|p| p.sandbox.is_some()),
        profile: super::profiles::of(s, pane_id)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
    })
}

/// Emit a pane event of tilrd's core, scoped to the pane.
fn emit(s: &Supervisor, subject: &str, pane_id: PaneId, payload: Vec<u8>) {
    if let Some(broker) = s.bus.broker() {
        broker.emit(
            Identity::Tilrd,
            subject,
            payload.into(),
            Some(format!("pane:{pane_id}")),
        );
    }
}

/// A seq of the consumer on connection `conn` (`conn << 32` and up), else 0.
fn own_seq(seq: u64, conn: u64) -> u64 {
    if seq != 0 && seq >> 32 == conn {
        seq
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Add `pane.*` and the observer of the pane events.
pub fn register(registry: &mut Registry, core: &Core) {
    registry.observe(Arc::new(Observer));
    // Runs once the supervisor serves, after the panes of a reload are back: they
    // are not new, so no `pane.created` is told for them.
    let seeded = core.clone();
    let _ = core.supervisor.with(move |s| {
        let infos: Vec<_> = s
            .panes
            .keys()
            .filter_map(|&id| pane_info(s, id).map(|info| (id, info)))
            .collect();
        let st = state(s);
        st.core = Some(seeded);
        for (id, mut info) in infos {
            info.used_at_ms = 0;
            st.listed.insert(id, info);
        }
    });
    registry.add(create_method(core));
    registry.add(unary(core, methods::PANE_LIST.name, list));
    registry.add(unary(core, methods::PANE_CLOSE.name, close));
    registry.add(unary(core, methods::PANE_RESIZE.name, resize));
    registry.add(unary(core, methods::PANE_INPUT.name, input));
    registry.add(unary(core, methods::PANE_APPLIED.name, applied));
    registry.add(unary(
        core,
        methods::PANE_INPUT_FINALIZE.name,
        input_finalize,
    ));
    let reader = core.clone();
    registry.add(NativeMethod::unary(
        methods::PANE_READ.name,
        move |ctx, payload| read(&reader, ctx, &payload),
    ));
    let watcher = core.clone();
    registry.add(NativeMethod::stream(
        methods::PANE_OUTPUT.name,
        Lane::Interactive,
        move |ctx, payload, out| output(watcher.clone(), ctx, payload, out),
    ));
}

/// `pane.create`: a method of its own, given the call (`CallCtx`): the
/// authorizer and the resolution of `profile` (`profiles::prepare_create`) run
/// before the supervisor's part and hand it what the profile gives.
fn create_method(core: &Core) -> NativeMethod {
    let supervisor = core.supervisor.clone();
    let broker = core.broker.clone();
    NativeMethod::unary(methods::PANE_CREATE.name, move |ctx: CallCtx, payload| {
        let caller = Caller::of(&ctx);
        // The authorizer's answer and the profile, read now (P4), before the
        // supervisor's part is queued: the call keeps its place in the order of
        // its connection's calls (B5).
        let asked = json::decode::<pane::CreateRequest>(&ctx, &payload).and_then(|request| {
            let resolved =
                super::profiles::prepare_create(broker.authorizer().as_ref(), &ctx, &request)?;
            queue(&supervisor, move |s| create(s, &caller, request, resolved))
        });
        answered(ctx, asked)
    })
}

/// A unary method whose work runs on the supervisor's task. It is queued when
/// the call arrives, so calls from one connection run in the order sent (B5).
fn unary<Req, Resp, F>(core: &Core, method: &'static str, handler: F) -> NativeMethod
where
    Req: Message + Default + Json + Send + 'static,
    Resp: Message + Json + Send + 'static,
    F: Fn(&mut Supervisor, &Caller, Req) -> Answer<Resp> + Send + Sync + 'static,
{
    let supervisor = core.supervisor.clone();
    let handler = Arc::new(handler);
    NativeMethod::unary(method, move |ctx: CallCtx, payload: Bytes| {
        let caller = Caller::of(&ctx);
        let handler = Arc::clone(&handler);
        let asked = json::decode::<Req>(&ctx, &payload)
            .and_then(|request| queue(&supervisor, move |s| handler(s, &caller, request)));
        answered(ctx, asked)
    })
}

/// The reply of a call whose supervisor's part was queued (`asked`).
async fn answered<Resp: Message + Json>(
    ctx: CallCtx,
    asked: Result<oneshot::Receiver<Answer<Resp>>, BusError>,
) -> Result<Reply, BusError> {
    let gone = || BusError::unavailable("tilrd is shutting down");
    let response = match asked?.await.map_err(|_| gone())? {
        Answer::Now(result) => result?,
        Answer::Later(rx) => rx.await.map_err(|_| gone())??,
    };
    Ok(json::reply(&ctx, &response))
}

/// Queue `f` on the supervisor's task now and get what it returns later: as
/// `SupervisorHandle::ask`, whose future (edition 2024) keeps the handle
/// borrowed and so cannot leave a handler's synchronous part.
fn queue<R: Send + 'static>(
    supervisor: &super::SupervisorHandle,
    f: impl FnOnce(&mut Supervisor) -> R + Send + 'static,
) -> Result<oneshot::Receiver<R>, BusError> {
    let (tx, rx) = oneshot::channel();
    supervisor.with(move |s| {
        let _ = tx.send(f(s));
    })?;
    Ok(rx)
}

/// Runs `f` on the supervisor's task when dropped: the cleanup of a handler,
/// which the broker aborts when its caller cancels.
struct OnDrop<F: FnOnce(&mut Supervisor) + Send + 'static> {
    supervisor: super::SupervisorHandle,
    f: Option<F>,
}

impl<F: FnOnce(&mut Supervisor) + Send + 'static> OnDrop<F> {
    fn new(core: &Core, f: F) -> Self {
        Self {
            supervisor: core.supervisor.clone(),
            f: Some(f),
        }
    }
}

impl<F: FnOnce(&mut Supervisor) + Send + 'static> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.f.take() {
            let _ = self.supervisor.with(f);
        }
    }
}

// ---------------------------------------------------------------------------
// The observer and the hooks
// ---------------------------------------------------------------------------

struct Observer;

impl BusObserver for Observer {
    /// The hot path: per watcher a clone of the packet into its queue; nothing
    /// when no `pane.output` stream and no waiting `pane.read` names the pane.
    fn pane_output(&self, s: &Supervisor, pane_id: &PaneId, head: FrameHead, packet: &Bytes) {
        let Some(st) = s.bus.slot_ref::<PanesState>() else {
            return;
        };
        if let Some((change, _)) = st.changes.get(pane_id) {
            change.send_modify(|change| change.frames += 1);
        }
        if let Some(watchers) = st.watchers.get(pane_id) {
            for watcher in watchers {
                watcher.offer(head, packet);
            }
        }
    }

    fn pane_event(&self, s: &Supervisor, event: &PaneEvent) {
        match event {
            PaneEvent::Activity { pane, activity } => {
                let kind = pane::PaneActivityKind::try_from(activity.kind)
                    .unwrap_or(pane::PaneActivityKind::Unspecified);
                let subject = match kind {
                    pane::PaneActivityKind::Bell => methods::events::PANE_BELL,
                    pane::PaneActivityKind::CommandFinished => {
                        methods::events::PANE_COMMAND_FINISHED
                    }
                    pane::PaneActivityKind::Unspecified => return,
                };
                let payload = pane::PaneActivity {
                    pane_id: pane.to_string(),
                    kind: kind as i32,
                    at_ms: activity.at_ms,
                };
                emit(s, subject, *pane, payload.encode_to_vec());
            }
            PaneEvent::Exited { pane, exit_code } => {
                let exited = pane::PaneExited {
                    pane_id: pane.to_string(),
                    exit_code: *exit_code,
                };
                emit(
                    s,
                    methods::events::PANE_EXITED,
                    *pane,
                    exited.encode_to_vec(),
                );
                let Some(st) = s.bus.slot_ref::<PanesState>() else {
                    return;
                };
                for watcher in st.watchers.get(pane).into_iter().flatten() {
                    let _ = watcher.shared.exit.set(*exit_code);
                    watcher.shared.wake.notify_one();
                }
                if let Some((change, _)) = st.changes.get(pane) {
                    change.send_modify(|change| change.exit = Some(*exit_code));
                }
                let (pane, exit_code) = (*pane, *exit_code);
                if let Some(core) = &st.core {
                    let _ = core.supervisor.with(move |s| on_exited(s, pane, exit_code));
                }
            }
            // Created, title and directory: `panes_changed`, which also sees size
            // and viewers.
            _ => {}
        }
    }
}

/// The pane is gone: answer what waited for it.
fn on_exited(s: &mut Supervisor, pane_id: PaneId, exit_code: i32) {
    let st = state(s);
    st.watchers.remove(&pane_id);
    st.creators.remove(&pane_id);
    st.listed.remove(&pane_id);
    super::profiles::forget(s, pane_id);
    let st = state(s);
    st.changes.remove(&pane_id);
    // A text request the pane process did not answer before it ended: the
    // pane's program ended (T9: `alive` false with its exit code).
    let unanswered: Vec<u64> = st
        .reads
        .iter()
        .filter(|(_, wait)| wait.pane == pane_id)
        .map(|(&id, _)| id)
        .collect();
    for id in unanswered {
        if let Some(wait) = st.reads.remove(&id) {
            let _ = wait.reply.send(Ok(pane::ReadReply {
                alive: false,
                exit_code: Some(exit_code),
                ..Default::default()
            }));
        }
    }
    st.delivered.retain(|_, panes| {
        panes.remove(&pane_id);
        !panes.is_empty()
    });
    for reply in st.closing.remove(&pane_id).into_iter().flatten() {
        let _ = reply.send(Ok(pane::CloseReply { exit_code }));
    }
    st.acks.retain(|(_, pane), waits| {
        if *pane != pane_id {
            return true;
        }
        for wait in waits.drain(..) {
            let _ = wait.reply.send(Err(no_such_pane(&pane_id.to_string())));
        }
        false
    });
    let probes: Vec<u64> = st
        .probes
        .iter_mut()
        .filter_map(|(&id, probe)| probe.waiting.remove(&pane_id).then_some(id))
        .collect();
    for id in probes {
        finish_probe(st, id);
    }
}

/// Hook in `broadcast_pane_list`: what changed in the list since it was last
/// told: `pane.created` for a new pane (and the reply of its `pane.create`),
/// `pane.changed` for a new size, title, directory or viewers, `pane.title`.
pub(crate) fn panes_changed(s: &mut Supervisor) {
    let infos: Vec<(PaneId, pane::PaneInfo)> = s
        .panes
        .keys()
        .filter_map(|&id| pane_info(s, id).map(|info| (id, info)))
        .collect();
    let mut told = Vec::new();
    let mut orphans = Vec::new();
    let st = state(s);
    st.listed
        .retain(|id, _| infos.iter().any(|(pane, _)| pane == id));
    for (id, info) in infos {
        let mut key = info.clone();
        key.used_at_ms = 0;
        match st.listed.insert(id, key.clone()) {
            None => {
                if let Some(reply) = st.creating.remove(&id)
                    && reply.send(Ok(info.clone())).is_err()
                {
                    // Its `pane.create` was cancelled or its connection ended
                    // while it started: nobody holds the pane, it is closed.
                    orphans.push(id);
                }
                told.push((methods::events::PANE_CREATED, id, info));
            }
            Some(old) if old != key => {
                let title = old.title != key.title;
                told.push((methods::events::PANE_CHANGED, id, info.clone()));
                if title {
                    told.push((PANE_TITLE, id, info));
                }
            }
            Some(_) => {}
        }
    }
    for (subject, id, info) in told {
        emit(s, subject, id, info.encode_to_vec());
    }
    for pane_id in orphans {
        tracing::info!(pane = %pane_id, "pane.create was cancelled, closing its pane");
        send_close(s, pane_id);
    }
}

/// Hook in `on_pane_link_closed`: the pane's process ended before its program
/// ran; its `pane.create` fails.
pub(crate) fn create_failed(s: &mut Supervisor, pane_id: PaneId, reason: &str) {
    super::profiles::forget(s, pane_id);
    let st = state(s);
    st.creators.remove(&pane_id);
    if let Some(reply) = st.creating.remove(&pane_id) {
        let _ = reply.send(Err(BusError::handler_error(format!(
            "create pane: {reason}"
        ))));
    }
}

/// Hook in `on_input_ack`: a pane process acknowledged delivery input or
/// answered a probe.
pub(crate) fn input_ack(s: &mut Supervisor, pane_id: PaneId, ack: &tilr_proto::PaneInputAck) {
    let st = state(s);
    if ack.probe != 0 {
        if let Some(probe) = st.probes.get_mut(&ack.probe)
            && probe.identity == ack.identity
            && probe.waiting.remove(&pane_id)
        {
            probe.applied.push(pane::InputAck {
                pane_id: pane_id.to_string(),
                seq: ack.seq,
            });
            finish_probe(st, ack.probe);
        }
        return;
    }
    let key = (ack.identity.clone(), pane_id);
    let Some(waits) = st.acks.get_mut(&key) else {
        return;
    };
    // Acknowledgements are cumulative: everything up to `seq` is applied.
    let pending = std::mem::take(waits);
    for wait in pending {
        if wait.seq <= ack.seq {
            let _ = wait.reply.send(Ok(pane::InputAck {
                pane_id: pane_id.to_string(),
                seq: wait.input_seq,
            }));
        } else {
            waits.push(wait);
        }
    }
    if waits.is_empty() {
        st.acks.remove(&key);
    }
}

/// Hook in the pane reader: the pane process's answer to a text request.
pub(crate) fn text_reply(pane_id: PaneId, packet: Bytes) -> CoreCmd {
    CoreCmd::With(Box::new(move |s| {
        let (Some(id), Some(&status)) = (
            packet.get(..8).and_then(|id| <[u8; 8]>::try_from(id).ok()),
            packet.get(8),
        ) else {
            tracing::warn!(pane = %pane_id, "bad text from pane process");
            return;
        };
        let body = &packet[9..];
        let reply = match status {
            link::TEXT_OK => match pane::ReadReply::decode(body) {
                Ok(reply) => Ok(reply),
                Err(err) => {
                    tracing::warn!(pane = %pane_id, "bad text from pane process: {err}");
                    Err(BusError::handler_error(
                        "pane.read: bad text from the pane process",
                    ))
                }
            },
            link::TEXT_TOO_LARGE => Err(BusError::new(
                codes::TOO_LARGE,
                format!("pane.read: {}", String::from_utf8_lossy(body)),
            )),
            _ => Err(BusError::handler_error(format!(
                "pane.read: {}",
                String::from_utf8_lossy(body)
            ))),
        };
        let st = state(s);
        if let Some(wait) = st.reads.remove(&u64::from_le_bytes(id)) {
            if wait.pane == pane_id {
                let _ = wait.reply.send(reply);
            } else {
                st.reads.insert(u64::from_le_bytes(id), wait);
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// pane.create, pane.list, pane.close
// ---------------------------------------------------------------------------

/// `pane.create`, authorized and its profile resolved by
/// `profiles::prepare_create`: `resolved` (a request with a profile) gives the
/// command, the directory and the variables the pane starts with, and the
/// snapshot it keeps.
fn create(
    s: &mut Supervisor,
    caller: &Caller,
    request: pane::CreateRequest,
    resolved: Option<super::profiles::Resolved>,
) -> Answer<pane::PaneInfo> {
    let size = |n: u32, default: u16| if n == 0 { u32::from(default) } else { n };
    let (command, cwd, env, snapshot) = match resolved {
        None => (request.command, request.cwd, Vec::new(), None),
        Some(resolved) => (
            resolved.command,
            resolved
                .cwd
                .map(|cwd| cwd.to_string_lossy().into_owned())
                .unwrap_or_default(),
            resolved
                .env
                .into_iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect(),
            Some(resolved.snapshot),
        ),
    };
    let create = tilr_proto::CreatePane {
        cols: size(request.cols, crate::pane::DEFAULT_COLS),
        rows: size(request.rows, crate::pane::DEFAULT_ROWS),
        command,
        cwd,
        request_id: 0,
    };
    let sandbox = snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.sandbox.as_ref());
    match start_pane(s, &create, caller.from.to_string(), &env, sandbox) {
        Ok((pane_id, rx)) => {
            if let Some(snapshot) = snapshot {
                super::profiles::remember(s, pane_id, snapshot);
            }
            Answer::Later(rx)
        }
        Err(err) => Answer::Now(Err(err)),
    }
}

/// Start a pane on the supervisor's path for `create` (its `request_id` is the
/// core's), created by `creator`, with `env` added to the environment every
/// pane gets, inside `sandbox` when its profile has one; answered with its
/// `PaneInfo` once its program runs, or with `handler_error` `create pane:
/// <reason>`.
pub(crate) fn start_pane(
    s: &mut Supervisor,
    create: &tilr_proto::CreatePane,
    creator: String,
    env: &[(OsString, OsString)],
    sandbox: Option<&crate::profile::SandboxSpec>,
) -> Result<(PaneId, oneshot::Receiver<Result<pane::PaneInfo, BusError>>), BusError> {
    applies(sandbox)?;
    let st = state(s);
    st.next_create += 1;
    let token = st.next_create;
    let create = tilr_proto::CreatePane {
        request_id: token,
        ..create.clone()
    };
    s.create_pane(NO_CLIENT, &create, env)
        .map_err(|err| BusError::handler_error(format!("create pane: {err:#}")))?;
    let pane_id = s
        .starting
        .iter()
        .find(|(_, starting)| starting.client_id == NO_CLIENT && starting.request_id == token)
        .map(|(id, _)| *id)
        .ok_or_else(|| BusError::handler_error("create pane: the pane is not starting"))?;
    let (tx, rx) = oneshot::channel();
    let st = state(s);
    st.creating.insert(pane_id, tx);
    st.creators.insert(pane_id, creator);
    Ok((pane_id, rx))
}

/// Fail closed: nothing on the start path applies a sandbox yet (`sandbox-linux`,
/// `sandbox-macos` do), so a pane asked to run in one does not start, whatever
/// `profiles::sandbox_enforceable` says; it never runs unconfined while its
/// `PaneInfo.sandboxed` says otherwise.
fn applies(sandbox: Option<&crate::profile::SandboxSpec>) -> Result<(), BusError> {
    match sandbox {
        None => Ok(()),
        Some(_) => Err(BusError::handler_error(
            "create pane: this server cannot apply a sandbox",
        )),
    }
}

fn list(s: &mut Supervisor, caller: &Caller, _: pane::ListRequest) -> Answer<pane::ListReply> {
    let mut panes: Vec<pane::PaneInfo> = s
        .panes
        .keys()
        .filter(|&&id| may_access(s, caller, id).is_ok())
        .filter_map(|&id| pane_info(s, id))
        .collect();
    // Oldest first, as `tilr.v1`'s list.
    panes.sort_by(|a, b| (a.created_at_ms, &a.pane_id).cmp(&(b.created_at_ms, &b.pane_id)));
    Answer::Now(Ok(pane::ListReply { panes }))
}

fn close(
    s: &mut Supervisor,
    caller: &Caller,
    request: pane::CloseRequest,
) -> Answer<pane::CloseReply> {
    let pane_id = match accessible_pane(s, caller, &request.pane_id) {
        Ok(id) => id,
        Err(err) => return Answer::Now(Err(err)),
    };
    let (tx, rx) = oneshot::channel();
    state(s).closing.entry(pane_id).or_default().push(tx);
    send_close(s, pane_id);
    Answer::Later(rx)
}

/// The pane process hangs up its shell and exits with it.
fn send_close(s: &Supervisor, pane_id: PaneId) {
    let packet = tilr_proto::encode(&ClientMessage::from(Msg::ClosePane(
        tilr_proto::ClosePane {
            pane_id: pane_id.to_string(),
        },
    )));
    if let Some(p) = s.panes.get(&pane_id) {
        let _ = p.to_pane.send((packet, None));
    }
}

// ---------------------------------------------------------------------------
// pane.input, pane.resize, pane.applied, pane.input_finalize
// ---------------------------------------------------------------------------

/// The delivery identity of numbered input of `identity` with `client_id`:
/// another hash than `tilr.v1`'s, so the two protocols never share counters.
fn numbered_identity(identity: &Identity, client_id: &str) -> String {
    use sha2::{Digest, Sha256};
    if client_id.is_empty() {
        return String::new();
    }
    let identity = identity.to_string();
    let mut digest = Sha256::new();
    digest.update(b"tilr.bus.v1\0");
    digest.update((identity.len() as u64).to_le_bytes());
    digest.update(identity.as_bytes());
    digest.update(client_id.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The caller's session, made on first use.
fn session<'a>(s: &'a mut Supervisor, caller: &Caller) -> &'a mut Session {
    let conn = caller.conn();
    if !state(s).sessions.contains_key(&conn) {
        let numbered = numbered_identity(&caller.from, &caller.principal.client_id);
        s.input_retention.reconnect(&numbered);
        let st = state(s);
        st.sessions.insert(
            conn,
            Session {
                numbered,
                plain: format!("bus:{conn}"),
                next_plain: 0,
            },
        );
        if let Some(core) = st.core.clone() {
            tokio::spawn(watch_end(core, conn));
        }
    }
    state(s)
        .sessions
        .get_mut(&conn)
        .expect("the session was just made")
}

/// Forget the session of `conn` once the broker tells its connection ended (at
/// once if it is not connected): its own identity at once, its `client_id`'s an
/// hour after its last connection (`expire_input`).
async fn watch_end(core: Core, conn: u64) {
    if let Some(mut state) = core.broker.watch_connection(conn) {
        // An error: the broker dropped the connection, which ended too.
        let _ = state
            .wait_for(|state| matches!(state, ConnState::Ended(_)))
            .await;
    }
    let _ = core.supervisor.with(move |s| connection_ended(s, conn));
}

fn connection_ended(s: &mut Supervisor, conn: u64) {
    if let Some(session) = state(s).sessions.remove(&conn) {
        s.forget_input(&session.plain);
        // Its calls waiting for an acknowledgement have nobody to answer.
        drop_acks(state(s), "the connection ended", |_, wait| {
            wait.conn == conn
        });
        let shared = state(s)
            .sessions
            .values()
            .any(|other| other.numbered == session.numbered);
        if !session.numbered.is_empty() && !shared {
            s.input_retention
                .disconnect(session.numbered, std::time::Instant::now());
        }
    }
    forget_delivered(s);
}

/// Drop the acknowledgement waits `which` picks, answering those whose caller
/// still waits with `unavailable` `why`.
fn drop_acks(st: &mut PanesState, why: &str, which: impl Fn(&(String, PaneId), &AckWait) -> bool) {
    st.acks.retain(|key, waits| {
        let (gone, kept): (Vec<_>, Vec<_>) = waits.drain(..).partition(|wait| which(key, wait));
        for wait in gone {
            let _ = wait.reply.send(Err(BusError::unavailable(format!(
                "pane.input: {why}; its acknowledgement does not come"
            ))));
        }
        *waits = kept;
        !waits.is_empty()
    });
}

/// Forget which panes the numbered identities that are gone (finalized,
/// expired) sent input to: only those of a session or kept for a reconnect
/// stay.
fn forget_delivered(s: &mut Supervisor) {
    let retained = &s.input_retention.disconnected;
    let Some(st) = s.bus.slot_ref::<PanesState>() else {
        return;
    };
    let gone: Vec<String> = st
        .delivered
        .keys()
        .filter(|identity| {
            !retained.contains_key(*identity)
                && !st
                    .sessions
                    .values()
                    .any(|session| &session.numbered == *identity)
        })
        .cloned()
        .collect();
    let st = state(s);
    for identity in gone {
        st.delivered.remove(&identity);
    }
}

/// Send input to the pane process as delivery input and wait for its
/// acknowledgement. `received`: it is input to count (`None` for a resize).
fn deliver(
    s: &mut Supervisor,
    caller: &Caller,
    pane_id: PaneId,
    msg: Msg,
    seq: u64,
    input_seq: u64,
    received: Option<std::time::Instant>,
) -> Answer<pane::InputAck> {
    let session = session(s, caller);
    let (identity, delivery_seq) = if input_seq > 0 {
        (session.numbered.clone(), input_seq)
    } else {
        session.next_plain += 1;
        (session.plain.clone(), session.next_plain)
    };
    let packet = tilr_proto::encode(&ClientMessage {
        msg: Some(msg),
        seq,
        input_seq: delivery_seq,
        input_identity: identity.clone(),
        ..Default::default()
    });
    let (tx, rx) = oneshot::channel();
    let st = state(s);
    if input_seq > 0 {
        st.delivered
            .entry(identity.clone())
            .or_default()
            .insert(pane_id);
    }
    let waits = st.acks.entry((identity, pane_id)).or_default();
    // Calls cancelled (or past their deadline) while their input waited.
    waits.retain(|wait| !wait.reply.is_closed());
    waits.push(AckWait {
        conn: caller.conn(),
        seq: delivery_seq,
        input_seq,
        reply: tx,
    });
    if let Some(p) = s.panes.get_mut(&pane_id) {
        if received.is_some() {
            p.counters.input += 1;
            p.counters.input_bytes += packet.len() as u64;
            p.used_at_ms = now_ms();
        }
        let _ = p.to_pane.send((packet, received));
    }
    Answer::Later(rx)
}

/// The checks of T5 before input is taken: numbered input needs a `client_id`
/// (and a session not finalized), then the pane must run and be the caller's.
fn input_target(
    s: &mut Supervisor,
    caller: &Caller,
    named: &str,
    input_seq: u64,
) -> Result<PaneId, BusError> {
    if input_seq > 0 && session(s, caller).numbered.is_empty() {
        return Err(BusError::bad_request(
            "numbered input requires a stable client_id",
        ));
    }
    accessible_pane(s, caller, named)
}

fn input(
    s: &mut Supervisor,
    caller: &Caller,
    request: pane::InputRequest,
) -> Answer<pane::InputAck> {
    use pane::input_request::Input;
    let received = std::time::Instant::now();
    let Some(input) = request.input else {
        return Answer::Now(Err(BusError::bad_request(
            "pane.input: one of key, mouse, paste, raw, scroll, clear is needed",
        )));
    };
    let pane_id = match input_target(s, caller, &request.pane_id, request.input_seq) {
        Ok(id) => id,
        Err(err) => return Answer::Now(Err(err)),
    };
    let named = pane_id.to_string();
    let msg = match input {
        Input::Key(key) => Msg::Key(tilr_proto::KeyInput {
            pane_id: named,
            action: key.action,
            mods: key.mods,
            named: key.named,
            codepoint: key.codepoint,
            text: key.text,
            code: key.code,
        }),
        Input::Mouse(mouse) => Msg::Mouse(tilr_proto::MouseInput {
            pane_id: named,
            action: mouse.action,
            button: mouse.button,
            mods: mouse.mods,
            x: mouse.x,
            y: mouse.y,
        }),
        Input::Paste(paste) => Msg::Paste(tilr_proto::Paste {
            pane_id: named,
            text: paste.text,
        }),
        Input::Raw(raw) => Msg::Raw(tilr_proto::RawInput {
            pane_id: named,
            data: raw.data,
        }),
        Input::Scroll(scroll) => Msg::Scroll(tilr_proto::Scroll {
            pane_id: named,
            delta: scroll.delta,
            to_bottom: scroll.to_bottom,
        }),
        Input::Clear(clear) => Msg::ClearPane(tilr_proto::ClearPane {
            pane_id: named,
            history: clear.history,
        }),
    };
    deliver(
        s,
        caller,
        pane_id,
        msg,
        request.seq,
        request.input_seq,
        Some(received),
    )
}

fn resize(
    s: &mut Supervisor,
    caller: &Caller,
    request: pane::ResizeRequest,
) -> Answer<pane::InputAck> {
    let pane_id = match input_target(s, caller, &request.pane_id, request.input_seq) {
        Ok(id) => id,
        Err(err) => return Answer::Now(Err(err)),
    };
    let clamp = |n: u32| n.clamp(1, u32::from(u16::MAX)) as u16;
    // As `tilr.v1`: numbered resizes report their size in the acknowledgement.
    if request.input_seq == 0
        && let Some(p) = s.panes.get_mut(&pane_id)
    {
        p.cols = clamp(request.cols);
        p.rows = clamp(request.rows);
    }
    let msg = Msg::Resize(tilr_proto::Resize {
        pane_id: pane_id.to_string(),
        cols: request.cols,
        rows: request.rows,
        cell_width_px: request.cell_width_px,
        cell_height_px: request.cell_height_px,
    });
    let answer = deliver(
        s,
        caller,
        pane_id,
        msg,
        request.seq,
        request.input_seq,
        None,
    );
    if request.input_seq == 0 {
        s.broadcast_pane_list();
    }
    answer
}

fn applied(
    s: &mut Supervisor,
    caller: &Caller,
    _: pane::AppliedRequest,
) -> Answer<pane::AppliedReply> {
    let identity = session(s, caller).numbered.clone();
    // Only the panes it sent numbered input to: a pane it never sent any to has
    // no counter of it, and a stuck one of them is not closed for its sake.
    let sent: Vec<PaneId> = state(s)
        .delivered
        .get(&identity)
        .into_iter()
        .flatten()
        .copied()
        .collect();
    let waiting: HashSet<PaneId> = sent
        .into_iter()
        .filter(|id| s.panes.contains_key(id) && may_access(s, caller, *id).is_ok())
        .collect();
    if waiting.is_empty() {
        return Answer::Now(Ok(pane::AppliedReply::default()));
    }
    let st = state(s);
    st.next_probe += 1;
    let probe = PROBE_BASE + st.next_probe;
    let (tx, rx) = oneshot::channel();
    let ask = tilr_proto::encode(&ClientMessage {
        input_identity: identity.clone(),
        input_probe: probe,
        ..Default::default()
    });
    for pane_id in &waiting {
        if let Some(p) = s.panes.get(pane_id) {
            let _ = p.to_pane.send((ask.clone(), None));
        }
    }
    let st = state(s);
    st.probes.insert(
        probe,
        Probe {
            identity,
            waiting,
            applied: Vec::new(),
            reply: Some(tx),
        },
    );
    if let Some(core) = st.core.clone() {
        tokio::spawn(async move {
            tokio::time::sleep(HELLO_PROBE_TIMEOUT).await;
            let _ = core.supervisor.with(move |s| probe_expired(s, probe));
        });
    }
    Answer::Later(rx)
}

/// A pane that cannot report its counters within 5 s is closed; the answer
/// comes once it is gone, without it (as `tilr.v1`'s Hello).
fn probe_expired(s: &mut Supervisor, probe: u64) {
    let Some(waiting) = state(s)
        .probes
        .get(&probe)
        .map(|probe| probe.waiting.clone())
    else {
        return;
    };
    for pane_id in waiting {
        if let Some(p) = s.panes.get_mut(&pane_id)
            && !p.probe_killed
        {
            p.probe_killed = true;
            tracing::error!(pane = %pane_id, "pane process did not answer pane.applied, closing it");
            // SAFETY: kill has no memory-safety preconditions.
            unsafe {
                libc::kill(p.pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

fn finish_probe(st: &mut PanesState, id: u64) {
    if !st.probes.get(&id).is_some_and(|p| p.waiting.is_empty()) {
        return;
    }
    let Some(mut probe) = st.probes.remove(&id) else {
        return;
    };
    probe.applied.sort_by(|a, b| a.pane_id.cmp(&b.pane_id));
    if let Some(reply) = probe.reply.take() {
        let _ = reply.send(Ok(pane::AppliedReply {
            applied: probe.applied,
        }));
    }
}

fn input_finalize(
    s: &mut Supervisor,
    caller: &Caller,
    _: pane::InputFinalizeRequest,
) -> Answer<pane::InputFinalizeReply> {
    // Numbered input on this connection is refused from now on, as on a
    // connection without a `client_id`.
    let identity = std::mem::take(&mut session(s, caller).numbered);
    s.input_retention.reconnect(&identity);
    s.forget_input(&identity);
    if !identity.is_empty() {
        // The pane processes forget its acknowledgements too.
        let st = state(s);
        drop_acks(st, "the input session was finalized", |(of, _), _| {
            *of == identity
        });
        st.delivered.remove(&identity);
    }
    Answer::Now(Ok(pane::InputFinalizeReply {}))
}

// ---------------------------------------------------------------------------
// pane.output
// ---------------------------------------------------------------------------

fn viewer(principal: &Principal) -> pane::Viewer {
    // The principal does not carry the peer's address; the transport says where.
    let address = match principal.transport {
        Transport::UnixSocket => "unix",
        Transport::Tcp => "tcp",
        Transport::InstanceFd | Transport::InstanceSocket => "instance",
    };
    pane::Viewer {
        client_id: principal.client_id.clone(),
        client_name: principal.client_name.clone(),
        address: address.into(),
        identity: principal.identity.to_string(),
    }
}

/// Register a stream of the pane's frames and ask the pane for a full frame.
fn start_watching(
    s: &mut Supervisor,
    caller: &Caller,
    request: &pane::OutputRequest,
    token: u64,
    feed: mpsc::Sender<FrameHeadPacket>,
    shared: Arc<WatchShared>,
) -> Result<PaneId, BusError> {
    let pane_id = accessible_pane(s, caller, &request.pane_id)?;
    state(s).watchers.entry(pane_id).or_default().push(Watcher {
        token,
        viewer: viewer(&caller.principal),
        feed,
        shared,
        conn: caller.conn(),
    });
    let Some(p) = s.panes.get_mut(&pane_id) else {
        return Err(no_such_pane(&request.pane_id));
    };
    p.used_at_ms = now_ms();
    if request.cell_width_px > 0 && request.cell_height_px > 0 {
        let resize = ClientMessage::from(Msg::Resize(tilr_proto::Resize {
            pane_id: pane_id.to_string(),
            cols: u32::from(p.cols),
            rows: u32::from(p.rows),
            cell_width_px: request.cell_width_px,
            cell_height_px: request.cell_height_px,
        }));
        let _ = p.to_pane.send((tilr_proto::encode(&resize), None));
    }
    // A subscribe is what makes a pane send a full frame, as for `tilr.v1`.
    let ask = ClientMessage::from(Msg::Subscribe(tilr_proto::Subscribe {
        pane_id: pane_id.to_string(),
    }));
    let _ = p.to_pane.send((tilr_proto::encode(&ask), None));
    p.full_asked = true;
    panes_changed(s);
    Ok(pane_id)
}

/// The stream `token` is over: its caller is no viewer any more.
fn unwatch(s: &mut Supervisor, token: u64) {
    let st = state(s);
    let Some(pane_id) = st
        .watchers
        .iter()
        .find(|(_, watchers)| watchers.iter().any(|w| w.token == token))
        .map(|(&id, _)| id)
    else {
        return;
    };
    if let Some(watchers) = st.watchers.get_mut(&pane_id) {
        watchers.retain(|w| w.token != token);
        if watchers.is_empty() {
            st.watchers.remove(&pane_id);
        }
    }
    if s.panes.contains_key(&pane_id) {
        panes_changed(s);
    }
}

async fn output(core: Core, ctx: CallCtx, payload: Bytes, out: StreamOut) {
    let request = match json::decode::<pane::OutputRequest>(&ctx, &payload) {
        Ok(request) => request,
        Err(err) => return out.fail(err),
    };
    let caller = Caller::of(&ctx);
    let conn = caller.conn();
    let shared = Arc::new(WatchShared::default());
    let (feed, frames) = mpsc::channel(FEED);
    // The broker aborts the handler when the caller cancels, also before the
    // supervisor's part is back: the guard exists before it runs, and the
    // watcher goes with it.
    let token = next_token();
    let _unwatch = OnDrop::new(&core, move |s| unwatch(s, token));
    let watching = {
        let shared = Arc::clone(&shared);
        let request = request.clone();
        core.supervisor
            .ask(move |s| start_watching(s, &caller, &request, token, feed, shared))
            .await
    };
    let pane_id = match watching {
        Ok(Ok(pane_id)) => pane_id,
        Ok(Err(err)) | Err(err) => return out.fail(err),
    };
    let ended = Watch {
        core: &core,
        out: &out,
        shared: &shared,
        pacing: Pacing::new(pane_id, conn),
        need: 1,
        sent: 0,
        unsent: None,
    }
    .run(frames)
    .await;
    match ended {
        Ended::Exit(exit_code) => out.end(
            pane::PaneExited {
                pane_id: pane_id.to_string(),
                exit_code,
            }
            .encode_to_vec()
            .into(),
        ),
        Ended::Failed(err) => out.fail(err),
        // Cancelled or gone: nobody hears the end.
        Ended::Gone => out.end(Bytes::new()),
    }
}

/// How a `pane.output` stream ends.
enum Ended {
    /// The program exited with this code.
    Exit(i32),
    /// Cancelled, or its reader gone.
    Gone,
    /// A frame cannot go in the stream's window.
    Failed(BusError),
}

/// What became of a frame offered to the stream.
enum Step {
    /// Sent or skipped: go on.
    Go,
    Closed,
    Failed(BusError),
}

/// Which frames of a pane a stream passes on (T4), apart from sending them:
/// what the frames, the queue and the consumer's lag decide, so a test drives it
/// with a queue filled as the observer fills it.
struct Pacing {
    pane_id: PaneId,
    conn: u64,
    /// No full frame sent yet: frames before it are skipped.
    waiting_full: bool,
    /// The consumer lags: frames are skipped until a full one.
    lagging: bool,
    /// A full frame was asked for since it caught up.
    asked: bool,
    /// The latest of its own seqs among the frames skipped.
    own_seq: u64,
}

/// What [`Pacing::admit`] makes of a frame.
#[derive(Debug, PartialEq, Eq)]
enum Admit {
    /// Skipped (latest-wins).
    Skip,
    /// To be sent, with `Frame.last_input_seq` set to `patch` if given;
    /// `carried` is the consumer's latest echoed seq, should it not get out.
    Send { patch: Option<u64>, carried: u64 },
}

impl Pacing {
    fn new(pane_id: PaneId, conn: u64) -> Self {
        Self {
            pane_id,
            conn,
            waiting_full: true,
            lagging: false,
            asked: false,
            own_seq: 0,
        }
    }

    /// A frame did not fit the queue: skipping, and a full frame is asked for
    /// again once the consumer caught up, also if the frame missed was the full
    /// one it waited for.
    fn take_missed(&mut self, shared: &WatchShared) {
        if shared.missed.swap(false, Ordering::Acquire) {
            let seq = shared.missed_seq.swap(0, Ordering::Relaxed);
            if !self.lagging {
                tracing::debug!(pane = %self.pane_id, connection = self.conn, "pane.output consumer lags, skipping frames");
            }
            self.lagging = true;
            self.asked = false;
            self.own_seq = self.own_seq.max(own_seq(seq, self.conn));
        }
    }

    /// Start or go on skipping frames; `seq` is the skipped frame's echo.
    fn skip(&mut self, seq: u64) {
        if !self.lagging && !self.waiting_full {
            tracing::debug!(pane = %self.pane_id, connection = self.conn, "pane.output consumer lags, skipping frames");
            self.asked = false;
        }
        if !self.waiting_full {
            self.lagging = true;
        }
        self.own_seq = self.own_seq.max(own_seq(seq, self.conn));
    }

    /// Frame `n` of the queue: send it or skip it. A delta goes while the
    /// consumer keeps up (`consumer_lags` false); a full frame always goes, and
    /// ends the lag only if it is newer than every frame the queue missed: one
    /// queued before a missed frame shows an older screen than the deltas
    /// after the missed one assume, so a newer full frame is still due.
    fn admit(
        &mut self,
        shared: &WatchShared,
        n: u64,
        head: FrameHead,
        consumer_lags: impl FnOnce() -> bool,
    ) -> Admit {
        self.take_missed(shared);
        let own = own_seq(head.last_input_seq, self.conn);
        // What the consumer echoed last, should this frame not get out either.
        let carried = self.own_seq.max(own);
        if head.full {
            let patch = (self.own_seq != 0 && carried != head.last_input_seq).then_some(carried);
            self.waiting_full = false;
            if n > shared.missed_upto.load(Ordering::Acquire) {
                self.lagging = false;
                self.own_seq = 0;
            } else {
                if !self.lagging {
                    self.asked = false;
                }
                self.lagging = true;
                self.own_seq = carried;
            }
            return Admit::Send { patch, carried };
        }
        if self.waiting_full || self.lagging {
            self.own_seq = self.own_seq.max(own);
            return Admit::Skip;
        }
        if consumer_lags() {
            self.skip(head.last_input_seq);
            return Admit::Skip;
        }
        Admit::Send {
            patch: None,
            carried,
        }
    }
}

/// One stream's state of T4.
struct Watch<'a> {
    core: &'a Core,
    out: &'a StreamOut,
    shared: &'a WatchShared,
    pacing: Pacing,
    /// The bytes of the chunk that did not get out: the credit to wait for
    /// before a full frame is asked for, so a frame over the credit left is not
    /// asked for again and again.
    need: u64,
    /// The number of the last frame sent.
    sent: u64,
    /// The latest full frame that did not get out (no room, no credit).
    unsent: Option<Unsent>,
}

impl Watch<'_> {
    /// Until the program exits or the stream is over.
    async fn run(mut self, mut frames: mpsc::Receiver<FrameHeadPacket>) -> Ended {
        loop {
            self.pacing.take_missed(self.shared);
            if self.shared.exit.get().is_some() {
                return self.finish(&mut frames).await;
            }
            let ask = self.pacing.lagging && !self.pacing.asked;
            tokio::select! {
                biased;
                () = self.out.cancelled().cancelled() => return Ended::Gone,
                frame = frames.recv() => match frame {
                    Some((n, head, packet)) => match self.frame(n, head, &packet).await {
                        Step::Go => {}
                        Step::Closed => return Ended::Gone,
                        Step::Failed(err) => return Ended::Failed(err),
                    },
                    // The watchers go after the exit is set (`on_exited`).
                    None => return self.finish(&mut frames).await,
                },
                () = self.shared.wake.notified() => {}
                // Credit for the chunk that did not fit, not a byte (T4).
                () = self.out.caught_up_for(self.need), if ask => {
                    self.pacing.asked = true;
                    let pane_id = self.pacing.pane_id;
                    let _ = self.core.supervisor.with(move |s| {
                        if let Some(p) = s.panes.get_mut(&pane_id) {
                            ask_full_frame(pane_id, p);
                        }
                    });
                }
            }
        }
    }

    /// The program ended: what the pane showed last comes before the end. Its
    /// last frame is full; a consumer still behind then gets it whole, waiting
    /// for credit if it must.
    async fn finish(&mut self, frames: &mut mpsc::Receiver<FrameHeadPacket>) -> Ended {
        let Some(&exit_code) = self.shared.exit.get() else {
            return Ended::Gone;
        };
        while let Ok((n, head, packet)) = frames.try_recv() {
            match self.frame(n, head, &packet).await {
                Step::Go => {}
                Step::Closed => return Ended::Gone,
                Step::Failed(err) => return Ended::Failed(err),
            }
        }
        self.pacing.take_missed(self.shared);
        if !(self.pacing.waiting_full || self.pacing.lagging) {
            return Ended::Exit(exit_code);
        }
        let missed = self
            .shared
            .missed_full
            .lock()
            .ok()
            .and_then(|mut kept| kept.take());
        let last = [self.unsent.take(), missed]
            .into_iter()
            .flatten()
            .max_by_key(|(n, _, _)| *n)
            .filter(|(n, _, _)| *n > self.sent);
        if let Some((_, head, packet)) = last {
            let carried = self
                .pacing
                .own_seq
                .max(own_seq(head.last_input_seq, self.pacing.conn));
            let patch = (carried != 0 && carried != head.last_input_seq).then_some(carried);
            match output_chunks(&packet, patch, self.limit()) {
                Ok(chunks) => {
                    for chunk in chunks {
                        if !self.send(chunk).await {
                            return Ended::Gone;
                        }
                    }
                }
                Err(FrameError::TooLarge(err)) => return Ended::Failed(too_large(&err)),
                Err(FrameError::Bad(err)) => {
                    tracing::warn!(pane = %self.pacing.pane_id, "last frame not sent on the bus: {err}");
                }
            }
        }
        Ended::Exit(exit_code)
    }

    /// The largest chunk this stream takes: its window, at most `max_frame`
    /// less the envelope.
    fn limit(&self) -> usize {
        usize::try_from(self.out.window()).map_or(MAX_CHUNK, |window| window.min(MAX_CHUNK))
    }

    /// Send `chunk`, waiting for credit; `false` once the stream is over.
    async fn send(&self, chunk: Chunk) -> bool {
        tokio::select! {
            biased;
            () = self.out.cancelled().cancelled() => false,
            sent = self.out.send(chunk) => sent.is_ok(),
        }
    }

    /// Pass frame `n` on or skip it. A frame goes whole or not at all: when its
    /// first chunk finds no credit it is skipped, the rest of a frame whose
    /// first chunk went waits for credit (a frame is at most a few windows,
    /// its images first).
    async fn frame(&mut self, n: u64, head: FrameHead, packet: &Bytes) -> Step {
        let out = self.out;
        let (patch, carried) = match self.pacing.admit(self.shared, n, head, || out.is_lagging()) {
            Admit::Skip => return Step::Go,
            Admit::Send { patch, carried } => (patch, carried),
        };
        let chunks = match output_chunks(packet, patch, self.limit()) {
            Ok(chunks) => chunks,
            Err(FrameError::TooLarge(err)) => return Step::Failed(too_large(&err)),
            Err(FrameError::Bad(err)) => {
                tracing::warn!(pane = %self.pacing.pane_id, "frame not sent on the bus: {err}");
                return Step::Go;
            }
        };
        let mut chunks = chunks.into_iter();
        if let Some(first) = chunks.next() {
            match self.out.try_send(first) {
                Ok(()) => {}
                // No credit: lagging, this frame skipped; a full frame is asked
                // for once there is credit for this chunk.
                Err(TrySendError::Full(chunk)) => {
                    self.pacing.skip(0);
                    self.pacing.own_seq = carried;
                    self.need = (chunk.payload.len() as u64).max(1);
                    if head.full {
                        self.unsent = Some((n, head, packet.clone()));
                    }
                    return Step::Go;
                }
                Err(TrySendError::Closed) => return Step::Closed,
            }
        }
        for chunk in chunks {
            if !self.send(chunk).await {
                return Step::Closed;
            }
        }
        self.need = 1;
        self.sent = n;
        Step::Go
    }
}

/// Why a frame packet makes no chunks.
#[derive(Debug)]
enum FrameError {
    /// Not a frame the core can read: skipped.
    Bad(String),
    /// The frame without its images is over the largest chunk: the stream
    /// cannot carry it.
    TooLarge(String),
}

impl From<String> for FrameError {
    fn from(err: String) -> Self {
        Self::Bad(err)
    }
}

impl From<&str> for FrameError {
    fn from(err: &str) -> Self {
        Self::Bad(err.into())
    }
}

fn too_large(err: &str) -> BusError {
    BusError::new(codes::TOO_LARGE, format!("pane.output: {err}"))
}

/// The chunks of `pane.output` for a frame packet of a pane process (`tilr.v1`
/// `ServerMessage{frame}`, compressed or not), with `Frame.last_input_seq` set
/// to `patch` if given, each at most `limit` bytes (the stream's window, at
/// most `max_frame` less the envelope). A frame over it goes as its images
/// first, then the frame without them; an image alone over it is left out.
fn output_chunks(
    packet: &Bytes,
    patch: Option<u64>,
    limit: usize,
) -> Result<Vec<Chunk>, FrameError> {
    let flag = *packet.first().ok_or("empty packet")?;
    if patch.is_none() {
        // The frame's own encoding, only the wrapping tag changed: a copy of a
        // small plain packet, the decompressed body of a large one.
        let plain = match flag {
            0 => Some(BytesMut::from(&packet[1..])),
            1 => tilr_bus::Chunk {
                payload: packet.slice(1..),
                content_encoding: tilr_bus::codec::ENCODING_ZSTD.into(),
            }
            .bytes()
            .ok()
            .map(BytesMut::from),
            other => return Err(format!("unknown packet flag {other}").into()),
        };
        if let Some(mut plain) = plain
            && plain.first() == Some(&SERVER_MESSAGE_FRAME_TAG)
            && plain.len() <= limit
        {
            plain[0] = PANE_OUTPUT_FRAME_TAG;
            return Ok(vec![Chunk::new(plain.freeze())]);
        }
    }
    let message = tilr_proto::decode::<ServerMessage>(packet)
        .map_err(|err| FrameError::Bad(err.to_string()))?;
    let Some(server_message::Msg::Frame(frame)) = message.msg else {
        return Err("not a frame".into());
    };
    let mut frame = pane::Frame::decode(frame.encode_to_vec().as_slice())
        .map_err(|err| FrameError::Bad(format!("frame: {err}")))?;
    if let Some(seq) = patch {
        frame.last_input_seq = seq;
    }
    let mut chunks = Vec::new();
    let whole = pane::PaneOutput {
        output: Some(pane::pane_output::Output::Frame(Box::new(frame))),
    };
    if whole.encoded_len() <= limit {
        chunks.push(Chunk::new(whole.encode_to_vec()));
        return Ok(chunks);
    }
    let Some(pane::pane_output::Output::Frame(mut frame)) = whole.output else {
        unreachable!("built as a frame");
    };
    let mut batch: Vec<pane::Image> = Vec::new();
    let mut size = 0;
    for image in std::mem::take(&mut frame.images) {
        let len = image.encoded_len() + 8;
        if len > limit {
            tracing::warn!(
                image = image.id,
                bytes = len,
                limit,
                "an image over the largest chunk of pane.output left out of a frame"
            );
            continue;
        }
        if size + len > limit && !batch.is_empty() {
            chunks.push(images_chunk(std::mem::take(&mut batch)));
            size = 0;
        }
        size += len;
        batch.push(image);
    }
    if !batch.is_empty() {
        chunks.push(images_chunk(batch));
    }
    let rest = pane::PaneOutput {
        output: Some(pane::pane_output::Output::Frame(frame)),
    };
    let len = rest.encoded_len();
    if len > limit {
        return Err(FrameError::TooLarge(format!(
            "a frame of {len} bytes without its images is over the largest chunk of the stream ({limit} bytes: its window, at most max_frame)"
        )));
    }
    chunks.push(Chunk::new(rest.encode_to_vec()));
    Ok(chunks)
}

fn images_chunk(images: Vec<pane::Image>) -> Chunk {
    Chunk::new(
        pane::PaneOutput {
            output: Some(pane::pane_output::Output::Images(pane::FrameImages {
                images,
            })),
        }
        .encode_to_vec(),
    )
}

// ---------------------------------------------------------------------------
// pane.read
// ---------------------------------------------------------------------------

/// A text request on its way: its id and where the answer comes.
type TextWait = (u64, oneshot::Receiver<Result<pane::ReadReply, BusError>>);

/// Ask the pane process for its screen as text, for the call `call`.
fn request_text(
    s: &mut Supervisor,
    pane_id: PaneId,
    history: u32,
    call: u64,
) -> Result<TextWait, BusError> {
    let Some(p) = s.panes.get(&pane_id) else {
        return Err(no_such_pane(&pane_id.to_string()));
    };
    let id = next_token();
    let mut packet = Vec::with_capacity(13);
    packet.push(link::READ_TEXT);
    packet.extend_from_slice(&id.to_le_bytes());
    packet.extend_from_slice(&history.to_le_bytes());
    let _ = p.to_pane.send((packet.into(), None));
    let (tx, rx) = oneshot::channel();
    state(s).reads.insert(
        id,
        ReadWait {
            pane: pane_id,
            call,
            reply: tx,
        },
    );
    Ok((id, rx))
}

type ReadStart = (PaneId, TextWait, Option<watch::Receiver<Change>>);

fn start_read(
    s: &mut Supervisor,
    caller: &Caller,
    named: &str,
    history: u32,
    wait: bool,
    call: u64,
) -> Result<ReadStart, BusError> {
    let pane_id = accessible_pane(s, caller, named)?;
    let change = wait.then(|| {
        let (change, calls) = state(s)
            .changes
            .entry(pane_id)
            .or_insert_with(|| (watch::channel(Change::default()).0, HashSet::new()));
        calls.insert(call);
        change.subscribe()
    });
    let text = request_text(s, pane_id, history, call)?;
    Ok((pane_id, text, change))
}

/// The call `call` is over: its text requests and its wait go.
fn end_read(s: &mut Supervisor, call: u64) {
    let st = state(s);
    st.reads.retain(|_, wait| wait.call != call);
    st.changes.retain(|_, (_, calls)| {
        calls.remove(&call);
        !calls.is_empty()
    });
}

/// `pane.read`. Its supervisor's part is queued when the call arrives, so it
/// runs in the order the connection sent its calls (B5); what it leaves there
/// goes with the guard, also when the broker aborts the handler (the caller
/// cancelled).
fn read(
    core: &Core,
    ctx: CallCtx,
    payload: &[u8],
) -> impl Future<Output = Result<Reply, BusError>> + Send + use<> {
    let call = next_token();
    let guard = OnDrop::new(core, move |s| end_read(s, call));
    let started = json::decode::<pane::ReadRequest>(&ctx, payload).and_then(|request| {
        let caller = Caller::of(&ctx);
        let named = request.pane_id.clone();
        let history = request.history_lines.min(MAX_HISTORY);
        let wait = request.since_seq.is_some();
        let asked = queue(&core.supervisor, move |s| {
            start_read(s, &caller, &named, history, wait, call)
        })?;
        Ok((request, history, asked))
    });
    let core = core.clone();
    async move {
        let _guard = guard;
        let (request, history, asked) = started?;
        let (pane_id, (id, text), change) = asked
            .await
            .map_err(|_| BusError::unavailable("tilrd is shutting down"))??;
        let first = text_of(&core, &ctx, id, text, None).await?;
        let answer = match change {
            None => first,
            Some(change) => {
                let wait = Wait {
                    core: &core,
                    ctx: &ctx,
                    pane_id,
                    history,
                    call,
                };
                wait.for_change(&request, first, change).await
            }
        };
        fits(json::reply(&ctx, &answer))
    }
}

fn text_too_large() -> BusError {
    BusError::new(
        codes::TOO_LARGE,
        "pane.read: the screen as text is over max_frame",
    )
}

/// The reply of `pane.read`, `too_large` past what the envelope takes: JSON
/// escapes a control character in six bytes, so a reply within it as protobuf
/// may not be as JSON.
fn fits(reply: Reply) -> Result<Reply, BusError> {
    if reply.payload.len() > MAX_CHUNK {
        return Err(text_too_large());
    }
    Ok(reply)
}

/// How long a text request may wait for the pane process: what the call has
/// left before its deadline, less [`DEADLINE_MARGIN`], or [`TEXT_WAIT`] without
/// a deadline; never past the end of a `since_seq` wait (`wait_left`).
fn text_wait(deadline_left: Option<Duration>, wait_left: Option<Duration>) -> Duration {
    let wait = deadline_left.map_or(TEXT_WAIT, |left| left.saturating_sub(DEADLINE_MARGIN));
    wait_left.map_or(wait, |left| wait.min(left))
}

/// The answer to text request `id`, within [`text_wait`] (`until`: the end of a
/// `since_seq` wait); `unavailable` when the pane process does not answer in
/// time, its request dropped.
async fn text_of(
    core: &Core,
    ctx: &CallCtx,
    id: u64,
    text: oneshot::Receiver<Result<pane::ReadReply, BusError>>,
    until: Option<Instant>,
) -> Result<pane::ReadReply, BusError> {
    let wait = text_wait(
        ctx.deadline
            .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now())),
        until.map(|until| until.saturating_duration_since(Instant::now())),
    );
    match tokio::time::timeout(wait, text).await {
        Ok(Ok(reply)) => {
            let reply = reply?;
            // The envelope must fit `max_frame`, JSON or not.
            if reply.encoded_len() > MAX_CHUNK {
                return Err(text_too_large());
            }
            Ok(reply)
        }
        Ok(Err(_)) => Err(BusError::unavailable("tilrd is shutting down")),
        Err(_) => {
            let _ = core.supervisor.with(move |s| {
                state(s).reads.remove(&id);
            });
            Err(BusError::unavailable(
                "pane.read: the pane process did not answer",
            ))
        }
    }
}

/// A `pane.read` with `since_seq`, waiting for a change.
struct Wait<'a> {
    core: &'a Core,
    ctx: &'a CallCtx,
    pane_id: PaneId,
    history: u32,
    call: u64,
}

impl Wait<'_> {
    /// T9: the first text after `since_seq`, or the last one when the program
    /// ended or the wait is over: at most `min(wait_ms, 25 s, the time the call
    /// has left - 100 ms)`. The text after a change is waited for no longer:
    /// at the limit the last text stands.
    async fn for_change(
        &self,
        request: &pane::ReadRequest,
        mut latest: pane::ReadReply,
        mut change: watch::Receiver<Change>,
    ) -> pane::ReadReply {
        let since = request.since_seq.unwrap_or(0);
        let mut wait = request.wait_ms.map_or(MAX_WAIT, |ms| {
            Duration::from_millis(u64::from(ms)).min(MAX_WAIT)
        });
        if let Some(deadline) = self.ctx.deadline {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            wait = wait.min(left.saturating_sub(DEADLINE_MARGIN));
        }
        let until = Instant::now() + wait;
        loop {
            if latest.seq > since || !latest.alive {
                return latest;
            }
            if let Some(exit_code) = change.borrow_and_update().exit {
                latest.alive = false;
                latest.exit_code = Some(exit_code);
                return latest;
            }
            if Instant::now() >= until {
                return latest;
            }
            tokio::select! {
                changed = change.changed() => {
                    if changed.is_err() {
                        return latest;
                    }
                }
                () = tokio::time::sleep_until(until) => return latest,
                () = self.ctx.cancel.cancelled() => return latest,
            }
            if change.borrow().exit.is_some() {
                continue;
            }
            let (pane_id, history, call) = (self.pane_id, self.history, self.call);
            let asked = self
                .core
                .supervisor
                .ask(move |s| request_text(s, pane_id, history, call))
                .await;
            // Not asked: the pane is gone, its exit is in `change` or comes.
            // Not answered in time: the last text stands.
            if let Ok(Ok((id, text))) = asked
                && let Ok(text) = text_of(self.core, self.ctx, id, text, Some(until)).await
            {
                if text.alive {
                    latest = text;
                } else {
                    // It ended before it answered: the last text, and the exit.
                    latest.alive = false;
                    latest.exit_code = text.exit_code;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_after_a_change_is_waited_for_within_the_reads_wait() {
        let s = Duration::from_secs;
        // No deadline: TEXT_WAIT, but not past the end of a `since_seq` wait.
        assert_eq!(text_wait(None, None), TEXT_WAIT);
        assert_eq!(text_wait(None, Some(s(1))), s(1));
        assert_eq!(text_wait(None, Some(Duration::ZERO)), Duration::ZERO);
        assert_eq!(text_wait(None, Some(s(30))), TEXT_WAIT);
        // A deadline: what is left of it less the margin, the wait's end still a cap.
        assert_eq!(text_wait(Some(s(10)), None), s(10) - DEADLINE_MARGIN);
        assert_eq!(text_wait(Some(s(10)), Some(s(2))), s(2));
        assert_eq!(text_wait(Some(s(2)), Some(s(10))), s(2) - DEADLINE_MARGIN);
        assert_eq!(text_wait(Some(DEADLINE_MARGIN), None), Duration::ZERO);
    }

    fn frame_packet(frame: tilr_proto::Frame) -> Bytes {
        tilr_proto::encode(&ServerMessage::from(server_message::Msg::Frame(frame)))
    }

    fn frame(rows: usize, last_input_seq: u64) -> tilr_proto::Frame {
        tilr_proto::Frame {
            pane_id: "p".into(),
            seq: 7,
            full: true,
            cols: 80,
            rows: rows as u32,
            changed: (0..rows)
                .map(|y| tilr_proto::Row {
                    y: y as u32,
                    runs: vec![tilr_proto::Run {
                        text: format!("row {y} {}", "x".repeat(60)),
                        ..Default::default()
                    }],
                })
                .collect(),
            last_input_seq,
            ..Default::default()
        }
    }

    fn decoded(chunk: &Chunk) -> pane::PaneOutput {
        pane::PaneOutput::decode(chunk.payload.as_ref()).unwrap()
    }

    #[test]
    fn a_frame_goes_on_the_bus_as_the_pane_encoded_it() {
        for rows in [1, 50] {
            // Small: a plain packet; large: compressed by the pane process.
            let original = frame(rows, 5);
            let packet = frame_packet(original.clone());
            assert_eq!(packet[0], u8::from(rows > 1), "flag of {rows} rows");
            let chunks = output_chunks(&packet, None, MAX_CHUNK).unwrap();
            assert_eq!(chunks.len(), 1);
            assert!(chunks[0].content_encoding.is_empty());
            // The bytes of the frame are the pane's: only the outer tag differs.
            let plain = tilr_proto::decode::<ServerMessage>(&packet)
                .unwrap()
                .encode_to_vec();
            assert_eq!(&chunks[0].payload[1..], &plain[1..]);
            let Some(pane::pane_output::Output::Frame(frame)) = decoded(&chunks[0]).output else {
                panic!("a frame");
            };
            assert_eq!(frame.encode_to_vec(), original.encode_to_vec());
        }
    }

    #[test]
    fn a_frame_after_lag_carries_the_consumers_seq() {
        let packet = frame_packet(frame(3, 9));
        let chunks = output_chunks(&packet, Some(42), MAX_CHUNK).unwrap();
        let Some(pane::pane_output::Output::Frame(frame)) = decoded(&chunks[0]).output else {
            panic!("a frame");
        };
        assert_eq!((frame.last_input_seq, frame.seq), (42, 7));
    }

    #[test]
    fn a_frame_over_max_frame_sends_its_images_first() {
        let image = |id| tilr_proto::Image {
            id,
            generation: 1,
            width: 1,
            height: 1,
            png: vec![id as u8; 6 << 20].into(),
        };
        let mut big = frame(2, 0);
        // Three images of 6 MiB: two chunks of images, then the frame; one of
        // 17 MiB is left out.
        big.images = vec![image(1), image(2), image(3)];
        big.images.push(tilr_proto::Image {
            png: vec![0; 17 << 20].into(),
            ..image(4)
        });
        let chunks = output_chunks(&frame_packet(big), None, MAX_CHUNK).unwrap();
        let outputs: Vec<_> = chunks.iter().map(decoded).collect();
        let ids: Vec<Vec<u32>> = outputs
            .iter()
            .map(|output| match &output.output {
                Some(pane::pane_output::Output::Images(images)) => {
                    images.images.iter().map(|image| image.id).collect()
                }
                Some(pane::pane_output::Output::Frame(frame)) => {
                    assert!(frame.images.is_empty());
                    Vec::new()
                }
                None => panic!("empty output"),
            })
            .collect();
        assert_eq!(ids, [vec![1, 2], vec![3], vec![]]);
        assert!(chunks.iter().all(|chunk| chunk.payload.len() <= MAX_CHUNK));
    }

    #[test]
    fn a_frame_is_split_to_the_streams_window() {
        let image = |id| tilr_proto::Image {
            id,
            generation: 1,
            width: 1,
            height: 1,
            png: vec![id as u8; 40 << 10].into(),
        };
        // A 64 KiB window: the images go in chunks of it, then the frame.
        let window = 64 << 10;
        let mut framed = frame(2, 0);
        framed.images = vec![image(1), image(2), image(3)];
        let chunks = output_chunks(&frame_packet(framed), None, window).unwrap();
        assert_eq!(chunks.len(), 4);
        assert!(chunks.iter().all(|chunk| chunk.payload.len() <= window));
        assert!(matches!(
            decoded(&chunks[3]).output,
            Some(pane::pane_output::Output::Frame(_))
        ));
        // A frame of text over the window cannot go: too_large, not skipped.
        let err = output_chunks(&frame_packet(frame(50, 0)), None, 1 << 10).unwrap_err();
        assert!(matches!(err, FrameError::TooLarge(_)), "{err:?}");
    }

    /// What a stream's task passes on of the frames in its queue, the queue
    /// filled by [`Watcher::offer`] as the observer does: frame numbers sent.
    fn drain(
        pacing: &mut Pacing,
        shared: &WatchShared,
        frames: &mut mpsc::Receiver<FrameHeadPacket>,
        sent: &mut Vec<(u64, bool)>,
    ) {
        pacing.take_missed(shared);
        while let Ok((n, head, _)) = frames.try_recv() {
            if let Admit::Send { .. } = pacing.admit(shared, n, head, || false) {
                sent.push((n, head.full));
            }
        }
    }

    #[test]
    fn a_full_frame_queued_before_a_missed_frame_does_not_end_the_lag() {
        let conn = 1;
        let shared = Arc::new(WatchShared::default());
        let (feed, mut frames) = mpsc::channel(FEED);
        let watcher = Watcher {
            token: 0,
            viewer: pane::Viewer::default(),
            feed,
            shared: Arc::clone(&shared),
            conn,
        };
        let full = FrameHead {
            full: true,
            last_input_seq: 0,
        };
        let delta = FrameHead::default();
        let packet = Bytes::new();
        let mut pacing = Pacing::new(uuid::Uuid::nil(), conn);
        let mut sent = Vec::new();
        // The stream starts: its full frame 1, then a delta.
        watcher.offer(full, &packet);
        watcher.offer(delta, &packet);
        drain(&mut pacing, &shared, &mut frames, &mut sent);
        // The task waits for credit: its queue fills with full frame 3 and
        // deltas 4..=18, delta 19 does not fit.
        watcher.offer(full, &packet);
        for _ in 4..=19 {
            watcher.offer(delta, &packet);
        }
        drain(&mut pacing, &shared, &mut frames, &mut sent);
        // Frame 3 went, yet it is older than the missed 19: still lagging, and
        // a full frame is to be asked for.
        assert!(pacing.lagging && !pacing.asked);
        // Delta 20 does not go on the screen of 3; the full frame 21 asked for
        // ends the lag.
        watcher.offer(delta, &packet);
        watcher.offer(full, &packet);
        watcher.offer(delta, &packet);
        drain(&mut pacing, &shared, &mut frames, &mut sent);
        assert!(!pacing.lagging);
        assert_eq!(
            sent,
            [(1, true), (2, false), (3, true), (21, true), (22, false)]
        );
        // Each delta sent follows the frame sent before it.
        for pair in sent.windows(2) {
            assert!(pair[1].1 || pair[1].0 == pair[0].0 + 1, "{sent:?}");
        }
    }

    #[test]
    fn a_text_reply_over_max_frame_as_json_is_too_large() {
        // Control characters: a byte each in protobuf, six in JSON.
        let text = pane::ReadReply {
            lines: vec!["\u{1}".repeat(4 << 20)],
            ..Default::default()
        };
        assert!(text.encoded_len() <= MAX_CHUNK);
        assert!(fits(Reply::encode(&text)).is_ok());
        let err = fits(Reply::new(text.to_json().to_string())).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            (
                codes::TOO_LARGE,
                "pane.read: the screen as text is over max_frame"
            )
        );
    }

    #[test]
    fn own_seqs_are_told_apart_by_connection() {
        assert_eq!(own_seq(7 << 32 | 3, 7), 7 << 32 | 3);
        assert_eq!(own_seq(8 << 32 | 3, 7), 0);
        assert_eq!(own_seq(0, 0), 0);
    }

    #[test]
    fn numbered_identities_differ_by_identity_and_client_id() {
        let owner = numbered_identity(&Identity::Owner, "c");
        assert_eq!(owner.len(), 64);
        assert_ne!(
            owner,
            numbered_identity(&Identity::Instance("c".into()), "c")
        );
        assert_ne!(owner, numbered_identity(&Identity::Owner, "d"));
        assert_eq!(numbered_identity(&Identity::Owner, ""), "");
    }

    #[test]
    fn the_start_path_refuses_a_sandbox_it_does_not_apply() {
        assert_eq!(applies(None), Ok(()));
        let refused = applies(Some(&crate::profile::SandboxSpec::default())).unwrap_err();
        assert_eq!(refused.code, tilr_bus::codes::HANDLER_ERROR);
        assert_eq!(
            refused.message,
            "create pane: this server cannot apply a sandbox"
        );
    }
}
