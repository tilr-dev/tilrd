//! The bus's transports and `Hello` (design B3, B4, B12). Owner: `bus-broker-core`,
//! then `bus-streams-events` (a call or an event refused instead of queued past
//! [`OPTIONAL_ROOM`], the interactive lane bounded in bytes too
//! ([`INTERACTIVE_BYTES`]), bulk envelopes counted ([`BULK_ENVELOPES`]), packets
//! encoded outside the broker's lock); `bus-web` keeps
//! its Origin rules in the authorizer.
//!
//! - WebSocket, on the TCP port and the Unix socket, for a handshake that offers the
//!   subprotocol `tilr.bus.v1` (`supervisor.rs` picks the route; a handshake without
//!   it is `tilr.v1`, served as before). One binary message is a flag byte and one
//!   envelope ([`tilr_bus::codec`]); a message that does not decode is answered
//!   `ConnectionError{bad_frame}` and the connection stays. The liveness of
//!   `tilr.v1` is kept: the server's pings, the silence timeout (4002), the full
//!   interactive lane (4001), superseding by `client_id` (4000).
//! - Stream sockets of instances ([`spawn_stream`]): a u32 big-endian length and one
//!   envelope; a bad length or an envelope that does not decode closes it.
//!
//! `Hello` comes first, with `protocol_version` 1; anything else first, another
//! version, a refusal of the authorizer or a reserved method provided: a
//! `ConnectionError` and the connection closed. Nothing is sent before `HelloAck`:
//! the connection is known to the broker only after it is queued.

use std::os::fd::{OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use tilr_bus::bus::{ConnectionError, Envelope, Hello, HelloAck, Lane, MethodSpec, envelope::Msg};
use tilr_bus::{PROTOCOL_VERSION, codec, codes, limits, names};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

use super::authz::{AuthzRequest, HelloContext};
use super::broker::{BrokerHandle, ConnId, ConnState};
use super::identity::{Peer, Principal, Seat, Transport};
use crate::outbox::{self, Outgoing, ToClient};
use crate::supervisor::{
    CLOSE_TIMEOUT, Closing, HELLO_TIMEOUT, PINGS_PER_TIMEOUT, finish_close, judge, limit_unsent,
    sleep_until, tick,
};

/// How a connection's envelopes are framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    WebSocket,
    Stream,
}

/// The most methods one `Hello` may provide: more is refused `bad_request`, so
/// that one participant cannot grow the routing table without bound.
pub const MAX_PROVIDES: usize = 1024;

/// Interactive envelopes queued for a connection past which a call is answered
/// `busy` and an event dropped instead of queued (design B5, B6): the rest of the
/// lane's 1024 is kept for what must be queued (replies, cancels, credits), so a
/// participant that is only slow is not closed 4001 for traffic it may refuse.
pub const OPTIONAL_ROOM: usize = 768;

/// Bytes of interactive envelopes queued for a connection and not yet written
/// past which it is too slow (closed 4001, as at 1024 envelopes): the lane is
/// bounded in bytes too, so a reader that stops reading interactive streams of
/// large chunks does not pin up to 1024 envelopes of 16 MiB. An envelope that
/// comes into an empty lane is always queued.
pub const INTERACTIVE_BYTES: usize = 2 * codec::MAX_FRAME;

/// [`OPTIONAL_ROOM`] in bytes: past it a call is `busy` and an event dropped.
pub const OPTIONAL_BYTES: usize = INTERACTIVE_BYTES / 4 * 3;

/// Bulk envelopes queued for a connection and not yet written past which the
/// broker queues no more bulk chunks (the connection window counts payload
/// bytes, this counts envelopes: small chunks do not fill the lane by the
/// million).
pub const BULK_ENVELOPES: usize = 1024;

/// Bulk bytes queued for a connection and not yet written past which the
/// broker queues no more bulk chunks (B7's 4 MiB bulk lane; a chunk that comes
/// into an empty lane always goes). The connection window alone does not bound
/// it: the window of a stream that ended goes back before its chunks are
/// written.
pub const BULK_BYTES: usize = crate::outbox::BULK_BYTES;

/// How [`Out::offer`] went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    Queued,
    /// The lane is behind: not queued.
    Full,
    /// The connection is gone.
    Gone,
}

/// The way to one connection: its two lanes (`outbox.rs`) and how to close it.
#[derive(Clone)]
pub struct Out {
    id: ConnId,
    tx: ToClient,
    close: mpsc::UnboundedSender<Closing>,
    framing: Framing,
    /// Notified when the interactive lane is written out after
    /// [`ToClient::want_drained`].
    drained: Arc<Notify>,
    /// Told to close already.
    closing: Arc<AtomicBool>,
    /// Interactive envelopes queued and not yet written (the writer counts down).
    queued: Arc<AtomicUsize>,
    /// Their bytes.
    queued_bytes: Arc<AtomicUsize>,
    /// Bulk envelopes queued and not yet written.
    bulk_queued: Arc<AtomicUsize>,
    /// Their bytes.
    bulk_bytes: Arc<AtomicUsize>,
}

impl Out {
    pub fn id(&self) -> ConnId {
        self.id
    }

    /// The envelope framed for this connection, or why it cannot be (over the
    /// frame limit), with no stand-in: a chunk that cannot be relayed ends its
    /// stream instead.
    pub fn try_encode(&self, msg: Msg) -> Result<Bytes, codec::CodecError> {
        let envelope = Envelope { msg: Some(msg) };
        match self.framing {
            Framing::WebSocket => codec::encode_ws(&envelope),
            Framing::Stream => codec::encode_stream(&envelope),
        }
    }

    /// The envelope framed for this connection; an answer over the limit becomes
    /// a `too_large` error reply of its id.
    pub fn encode(&self, msg: Msg) -> Option<Bytes> {
        let envelope = Envelope { msg: Some(msg) };
        let encoded = match self.framing {
            Framing::WebSocket => codec::encode_ws(&envelope),
            Framing::Stream => codec::encode_stream(&envelope),
        };
        match encoded {
            Ok(packet) => Some(packet),
            Err(err) => {
                // An answer over the limit is answered too_large instead.
                let id = match &envelope.msg {
                    Some(Msg::Reply(reply)) => Some(reply.id),
                    Some(Msg::Chunk(chunk)) => Some(chunk.id),
                    Some(Msg::End(end)) => Some(end.id),
                    _ => None,
                };
                tracing::warn!(target: "tilrd::bus", connection = self.id, "envelope not sent: {err}");
                let id = id?;
                let refusal = Envelope {
                    msg: Some(Msg::Reply(tilr_bus::bus::Reply {
                        id,
                        result: Some(tilr_bus::bus::reply::Result::Error(tilr_bus::bus::Error {
                            code: codes::TOO_LARGE.into(),
                            message: err.to_string(),
                        })),
                        content_type: String::new(),
                    })),
                };
                match self.framing {
                    Framing::WebSocket => codec::encode_ws(&refusal).ok(),
                    Framing::Stream => codec::encode_stream(&refusal).ok(),
                }
            }
        }
    }

    /// Queue an envelope without waiting: `false` if the connection is gone or
    /// stuck (a full interactive lane closes it 4001, as `tilr.v1`'s).
    pub fn send(&self, msg: Msg, lane: Lane) -> bool {
        let Some(packet) = self.encode(msg) else {
            return false;
        };
        self.send_packet(packet, lane)
    }

    /// [`send`](Self::send) of an envelope [`encode`](Self::encode)d already. In
    /// the bulk lane it is queued without waiting for room: the broker bounds what
    /// it queues there by the connection window.
    pub fn send_packet(&self, packet: Bytes, lane: Lane) -> bool {
        match lane {
            Lane::Bulk => {
                let len = packet.len();
                self.bulk_queued.fetch_add(1, Ordering::Relaxed);
                self.bulk_bytes.fetch_add(len, Ordering::Relaxed);
                if self.tx.queue_bulk_now(packet).is_ok() {
                    return true;
                }
                self.bulk_queued.fetch_sub(1, Ordering::Relaxed);
                self.bulk_bytes.fetch_sub(len, Ordering::Relaxed);
                false
            }
            Lane::Interactive => {
                let len = packet.len();
                // Counted before it is queued, so that the writer never counts
                // down below zero; given back if it is not.
                let bytes = self.queued_bytes.fetch_add(len, Ordering::Relaxed);
                self.queued.fetch_add(1, Ordering::Relaxed);
                if (bytes == 0 || bytes + len <= INTERACTIVE_BYTES)
                    && self.tx.try_send(Outgoing::from(packet)).is_ok()
                {
                    return true;
                }
                self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
                self.queued.fetch_sub(1, Ordering::Relaxed);
                if !self.tx.closed() && !self.closing.swap(true, Ordering::Relaxed) {
                    tracing::warn!(target: "tilrd::bus", connection = self.id, "participant too slow, closing");
                    let _ = self.close.send(Closing::Now(
                        tilr_bus::close::TOO_SLOW,
                        tilr_bus::close::TOO_SLOW_REASON,
                    ));
                }
                false
            }
        }
    }

    /// Queue an encoded call or event in the interactive lane unless the lane is
    /// behind: it holds [`OPTIONAL_ROOM`] envelopes or [`OPTIONAL_BYTES`], or
    /// (`lag`) has held one for over 250 ms. Never closes the connection.
    pub fn offer(&self, packet: Bytes, lag: bool) -> Offer {
        if self.tx.closed() {
            return Offer::Gone;
        }
        let len = packet.len();
        let bytes = self.queued_bytes.load(Ordering::Relaxed);
        if self.queued.load(Ordering::Relaxed) >= OPTIONAL_ROOM
            || (bytes > 0 && bytes + len > OPTIONAL_BYTES)
            || (lag && self.tx.lagging())
        {
            return Offer::Full;
        }
        // Counted before it is queued (the writer counts down once written).
        self.queued_bytes.fetch_add(len, Ordering::Relaxed);
        self.queued.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(Outgoing::from(packet)) {
            Ok(()) => Offer::Queued,
            Err(_) => {
                self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
                self.queued.fetch_sub(1, Ordering::Relaxed);
                if self.tx.closed() {
                    Offer::Gone
                } else {
                    Offer::Full
                }
            }
        }
    }

    /// Bulk envelopes queued and not yet written.
    pub fn bulk_queued(&self) -> usize {
        self.bulk_queued.load(Ordering::Relaxed)
    }

    /// Whether the bulk lane takes a packet of about `len` bytes now: fewer
    /// than [`BULK_ENVELOPES`] queued, and [`BULK_BYTES`] not passed (an empty
    /// lane takes any).
    pub fn bulk_room(&self, len: usize) -> bool {
        let bytes = self.bulk_bytes.load(Ordering::Relaxed);
        self.bulk_queued() < BULK_ENVELOPES && (bytes == 0 || bytes + len <= BULK_BYTES)
    }

    /// Interactive envelopes queued and not yet written.
    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    /// Queue an envelope in the bulk lane, waiting for room (4 MiB queued).
    pub async fn send_bulk(&self, msg: Msg) -> bool {
        let Some(packet) = self.encode(msg) else {
            return false;
        };
        let len = packet.len();
        self.bulk_queued.fetch_add(1, Ordering::Relaxed);
        self.bulk_bytes.fetch_add(len, Ordering::Relaxed);
        if self.tx.send_bulk(packet).await.is_ok() {
            return true;
        }
        self.bulk_queued.fetch_sub(1, Ordering::Relaxed);
        self.bulk_bytes.fetch_sub(len, Ordering::Relaxed);
        false
    }

    /// Close at once with a WebSocket close code.
    pub fn close(&self, code: u16, reason: &'static str) {
        self.closing.store(true, Ordering::Relaxed);
        let _ = self.close.send(Closing::Now(code, reason));
    }

    /// Write out what is queued (a `ConnectionError`), then close.
    pub fn flush_close(&self) {
        self.closing.store(true, Ordering::Relaxed);
        let _ = self.close.send(Closing::Flush);
    }

    pub fn lagging(&self) -> bool {
        self.tx.lagging()
    }

    pub fn idle(&self) -> bool {
        self.tx.idle()
    }

    /// Returns once the interactive lane is written out, or the connection gone.
    pub async fn caught_up(&self) {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            self.tx.want_drained();
            if self.tx.drained() || self.tx.closed() {
                return;
            }
            // Woken when the lane empties, or when the writer's queue is
            // dropped ([`WriterQueue`]): no timer needed.
            notified.await;
        }
    }
}

/// The writer's end of a connection's queue. Dropping it (the writer ended or
/// was aborted) closes the queue, then wakes [`Out::caught_up`], which then
/// sees the connection closed.
struct WriterQueue {
    outbox: Option<outbox::Outbox>,
    drained: Arc<Notify>,
}

impl WriterQueue {
    fn new(outbox: outbox::Outbox, out: &Out) -> Self {
        Self {
            outbox: Some(outbox),
            drained: Arc::clone(&out.drained),
        }
    }
}

impl std::ops::Deref for WriterQueue {
    type Target = outbox::Outbox;
    fn deref(&self) -> &outbox::Outbox {
        self.outbox.as_ref().expect("taken only on drop")
    }
}

impl std::ops::DerefMut for WriterQueue {
    fn deref_mut(&mut self) -> &mut outbox::Outbox {
        self.outbox.as_mut().expect("taken only on drop")
    }
}

impl Drop for WriterQueue {
    fn drop(&mut self) {
        drop(self.outbox.take());
        self.drained.notify_waiters();
    }
}

/// What a connection's writer counts down once it wrote a packet of `len` bytes;
/// a bulk lane that comes down to half of [`BULK_ENVELOPES`] or of
/// [`BULK_BYTES`], or empties, has the broker queue what waited for room in it.
struct Written {
    id: ConnId,
    broker: BrokerHandle,
    queued: Arc<AtomicUsize>,
    queued_bytes: Arc<AtomicUsize>,
    bulk_queued: Arc<AtomicUsize>,
    bulk_bytes: Arc<AtomicUsize>,
}

impl Written {
    fn new(out: &Out, broker: &BrokerHandle) -> Self {
        Self {
            id: out.id,
            broker: broker.clone(),
            queued: Arc::clone(&out.queued),
            queued_bytes: Arc::clone(&out.queued_bytes),
            bulk_queued: Arc::clone(&out.bulk_queued),
            bulk_bytes: Arc::clone(&out.bulk_bytes),
        }
    }

    fn wrote(&self, bulk: bool, len: usize) {
        if !bulk {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
        } else {
            let count = self.bulk_queued.fetch_sub(1, Ordering::Relaxed);
            let bytes = self.bulk_bytes.fetch_sub(len, Ordering::Relaxed);
            let half = BULK_BYTES / 2;
            // Half way down, or empty (a large chunk waits for an empty lane).
            if count == 1 || count == BULK_ENVELOPES / 2 || (bytes > half && bytes - len <= half) {
                self.broker.bulk_room(self.id);
            }
        }
    }
}

/// A connection's writer end and the queue it takes from.
pub(super) fn out(
    id: ConnId,
    framing: Framing,
) -> (Out, outbox::Outbox, mpsc::UnboundedReceiver<Closing>) {
    let (tx, outbox) = outbox::channel();
    let (close, closing) = mpsc::unbounded_channel();
    (
        Out {
            id,
            tx,
            close,
            framing,
            drained: Arc::new(Notify::new()),
            closing: Arc::new(AtomicBool::new(false)),
            queued: Arc::new(AtomicUsize::new(0)),
            queued_bytes: Arc::new(AtomicUsize::new(0)),
            bulk_queued: Arc::new(AtomicUsize::new(0)),
            bulk_bytes: Arc::new(AtomicUsize::new(0)),
        },
        outbox,
        closing,
    )
}

fn connection_error(code: &str, message: impl Into<String>) -> ConnectionError {
    ConnectionError {
        code: code.into(),
        message: message.into(),
    }
}

/// Who says `hello` and what it provides, or why it is turned away.
fn accept_hello(
    broker: &BrokerHandle,
    hello: &Hello,
    conn: &HelloContext<'_>,
) -> Result<(Principal, Vec<MethodSpec>), ConnectionError> {
    let principal = broker.authorizer().hello(hello, conn)?;
    if hello.provides.len() > MAX_PROVIDES {
        return Err(connection_error(
            codes::BAD_REQUEST,
            format!(
                "a Hello provides {} methods, at most {MAX_PROVIDES}",
                hello.provides.len()
            ),
        ));
    }
    for spec in &hello.provides {
        if !names::valid_method(&spec.method) {
            return Err(connection_error(
                codes::BAD_REQUEST,
                format!("{:?} is not a method name", spec.method),
            ));
        }
        if names::is_reserved(&spec.method) {
            return Err(connection_error(
                codes::RESERVED,
                format!(
                    "{} is in the namespace {} of tilrd",
                    spec.method,
                    names::namespace(&spec.method)
                ),
            ));
        }
        broker
            .authorizer()
            .authorize(
                &principal,
                None,
                &AuthzRequest::Provide {
                    method: &spec.method,
                },
            )
            .map_err(|denied| connection_error(codes::DENIED, denied.reason))?;
    }
    Ok((principal, hello.provides.clone()))
}

/// Greet a connection whose first envelope is `envelope`: `HelloAck` queued and
/// the connection registered, or a `ConnectionError` queued and the connection
/// closing. Returns whether it is greeted.
#[expect(
    clippy::too_many_arguments,
    reason = "the parts of a connection both transports pass"
)]
async fn greet(
    broker: &BrokerHandle,
    out: &Out,
    state: &watch::Sender<ConnState>,
    envelope: Envelope,
    transport: Transport,
    peer: Peer,
    origin: &str,
    mut seat: Option<Seat>,
) -> bool {
    let id = out.id();
    let refuse = |error: ConnectionError| {
        tracing::info!(target: "tilrd::bus::audit", connection = id, %peer, code = error.code, "refused: {}", error.message);
        ConnState::end(state, "refused", Some(error.clone()));
        out.send(Msg::ConnectionError(error), Lane::Interactive);
        out.flush_close();
        false
    };
    let Some(Msg::Hello(hello)) = envelope.msg else {
        return refuse(connection_error(
            codes::EXPECTED_HELLO,
            "expected Hello first",
        ));
    };
    let accepted = (|| {
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(connection_error(
                codes::UNSUPPORTED_VERSION,
                format!(
                    "unsupported protocol version {}: this server speaks {PROTOCOL_VERSION}",
                    hello.protocol_version
                ),
            ));
        }
        // The runner's check (the instance token) first; the authorizer sees
        // the seat without it.
        if let Some(check) = seat.as_mut().and_then(|seat| seat.check.take()) {
            check(&hello)?;
        }
        let context = HelloContext {
            transport,
            peer,
            origin,
            connection_id: id,
            seat: seat.as_ref(),
        };
        accept_hello(broker, &hello, &context)
    })();
    let (principal, provides) = match accepted {
        Ok(accepted) => accepted,
        Err(error) => return refuse(error),
    };
    let ack = HelloAck {
        protocol_version: PROTOCOL_VERSION,
        identity: principal.identity.to_string(),
        connection_id: id,
        max_frame: limits::MAX_FRAME,
        max_in_flight: limits::MAX_IN_FLIGHT,
        stream_window: limits::STREAM_WINDOW,
        connection_window: limits::CONNECTION_WINDOW,
        credential: broker
            .authorizer()
            .hello_ack_credential(&principal)
            .unwrap_or_default(),
        server_id: broker.server_id().to_owned(),
    };
    let identity = ack.identity.clone();
    let (client_id, name) = (principal.client_id.clone(), principal.client_name.clone());
    let count = provides.len();
    // Registered before it is announced: once a participant has its HelloAck, or
    // anybody saw the connection up, its methods are routed and listed.
    let up = broker
        .register(
            principal,
            provides,
            Msg::HelloAck(ack),
            out.clone(),
            state.clone(),
        )
        .await;
    if up {
        tracing::info!(
            target: "tilrd::bus",
            connection = id,
            %peer,
            identity,
            client_id,
            name,
            provides = count,
            "participant greeted"
        );
    } else {
        // A newer connection of the same client superseded it between its
        // registration and its announcement: it was never up, so it is not
        // logged greeted (the tests wait on that line to use a participant).
        tracing::info!(
            target: "tilrd::bus",
            connection = id,
            %peer,
            identity,
            client_id,
            name,
            "participant superseded before it was up"
        );
    }
    true
}

/// The WebSocket configuration of the bus: a message is a flag byte and an envelope
/// of up to 16 MiB, compressed or not (zstd's worst case adds a little), as the
/// client's; the codec enforces the exact limit. tungstenite's default frame limit
/// is exactly 16 MiB and would refuse an envelope at the limit.
pub fn ws_config() -> WebSocketConfig {
    let max = codec::MAX_FRAME + (1 << 20);
    WebSocketConfig::default()
        .max_message_size(Some(max))
        .max_frame_size(Some(max))
}

/// A WebSocket connection whose handshake chose `tilr.bus.v1`.
pub struct WsConn {
    pub id: ConnId,
    pub peer: Peer,
    pub transport: Transport,
    /// The socket, for the options and the reset on close.
    pub socket: RawFd,
    /// The handshake's `Origin`; empty without.
    pub origin: String,
}

/// Serve a bus participant on a WebSocket accepted with the handshake's
/// configuration (`tilr.v1`'s): it is taken over with [`ws_config`]. A client
/// sends nothing before the server's handshake response (RFC 6455 §4.1), so no
/// read-ahead is lost.
pub async fn serve_websocket<S>(ws: WebSocketStream<S>, conn: WsConn, broker: BrokerHandle)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let WsConn {
        id,
        peer,
        transport,
        socket,
        origin,
    } = conn;
    let ws =
        WebSocketStream::from_raw_socket(ws.into_inner(), Role::Server, Some(ws_config())).await;
    tracing::info!(target: "tilrd::bus", connection = id, %peer, "bus participant connected");
    if transport == Transport::Tcp {
        limit_unsent(socket);
    }
    let (mut sink, mut source) = ws.split();
    let (out, outbox, mut closing) = out(id, Framing::WebSocket);
    let mut outbox = WriterQueue::new(outbox, &out);
    let drained = Arc::clone(&out.drained);
    let written = Written::new(&out, &broker);
    let client_timeout = broker.client_timeout();
    let mut ping = client_timeout.map(|timeout| {
        let mut ping = tokio::time::interval(timeout / PINGS_PER_TIMEOUT);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ping
    });
    // Ends when the connection is to close, with whether the close went out.
    let mut writer = tokio::spawn(async move {
        let how = loop {
            let (message, outgoing) = tokio::select! {
                biased;
                how = closing.recv() => break how,
                packet = outbox.recv(true) => match packet {
                    Some(mut outgoing) => {
                        let packet = std::mem::take(&mut outgoing.packet);
                        let len = packet.len();
                        (Message::Binary(packet), Some((outgoing, len)))
                    }
                    None => break closing.try_recv().ok(),
                },
                () = tick(&mut ping) => (Message::Ping(Bytes::new()), None),
            };
            let send = sink.send(message);
            tokio::pin!(send);
            tokio::select! {
                sent = &mut send => if sent.is_err() {
                    return false;
                },
                how = closing.recv() => {
                    if !matches!(how, Some(Closing::Now(..))) {
                        let _ = tokio::time::timeout(CLOSE_TIMEOUT, send).await;
                    }
                    break how;
                }
            }
            if let Some((outgoing, len)) = outgoing {
                written.wrote(outgoing.bulk(), len);
                if outbox.written(outgoing) {
                    drained.notify_waiters();
                }
            }
        };
        let close = async {
            let frame = match how {
                Some(Closing::Now(code, reason)) => Some(CloseFrame {
                    code: code.into(),
                    reason: reason.into(),
                }),
                Some(Closing::Flush) => {
                    while let Some(Outgoing { packet, .. }) = outbox.try_recv() {
                        sink.send(Message::Binary(packet)).await?;
                    }
                    None
                }
                None => None,
            };
            sink.send(Message::Close(frame)).await
        };
        matches!(tokio::time::timeout(CLOSE_TIMEOUT, close).await, Ok(Ok(())))
    });

    let (state, _) = watch::channel(ConnState::AwaitingHello);
    let hello_by = tokio::time::Instant::now() + HELLO_TIMEOUT;
    let mut greeted = false;
    // A ConnectionError was sent: what comes until the close is not read.
    let mut refused = false;
    let mut heard = tokio::time::Instant::now();
    let mut answered = heard;
    let why = loop {
        let mut wake = None;
        if let Some(timeout) = client_timeout {
            match judge(heard, answered, timeout) {
                Ok(next) => wake = Some(next),
                Err(why) => break Some(why),
            }
        }
        if !greeted && !refused {
            if tokio::time::Instant::now() >= hello_by {
                refused = true;
                out.send(
                    Msg::ConnectionError(connection_error(
                        codes::EXPECTED_HELLO,
                        format!("no Hello within {} s", HELLO_TIMEOUT.as_secs()),
                    )),
                    Lane::Interactive,
                );
                out.flush_close();
                continue;
            }
            wake = Some(wake.map_or(hello_by, |wake: tokio::time::Instant| wake.min(hello_by)));
        }
        let message = tokio::select! {
            biased;
            closed = &mut writer => {
                finish_close(&mut source, socket, closed.unwrap_or(false)).await;
                break None;
            }
            message = source.next() => message,
            () = sleep_until(wake) => continue,
        };
        let Some(Ok(message)) = message else {
            break None;
        };
        heard = tokio::time::Instant::now();
        match message {
            Message::Binary(packet) if !refused => {
                let envelope = match codec::decode_ws(&packet) {
                    Ok(envelope) => envelope,
                    Err(err) if !greeted => {
                        // Not even a Hello: turned away as anything else first.
                        refused = true;
                        let error = connection_error(
                            codes::EXPECTED_HELLO,
                            format!("expected Hello first, got a bad message: {err}"),
                        );
                        ConnState::end(&state, "refused", Some(error.clone()));
                        out.send(Msg::ConnectionError(error), Lane::Interactive);
                        out.flush_close();
                        continue;
                    }
                    Err(err) => {
                        out.send(
                            Msg::ConnectionError(connection_error(
                                codes::BAD_FRAME,
                                format!("bad message: {err}"),
                            )),
                            Lane::Interactive,
                        );
                        continue;
                    }
                };
                if greeted {
                    broker.on_envelope(id, envelope);
                } else {
                    // Greeted once the core is in place (the server's start).
                    broker.ready().await;
                    greeted = greet(
                        &broker, &out, &state, envelope, transport, peer, &origin, None,
                    )
                    .await;
                    refused = !greeted;
                }
            }
            Message::Pong(_) => answered = heard,
            Message::Close(_) => break None,
            _ => {}
        }
    };
    broker.disconnect(id);
    ConnState::end(&state, "disconnected", None);
    tracing::info!(target: "tilrd::bus", connection = id, %peer, "bus participant gone");
    if why.is_some() {
        out.close(tilr_bus::close::TIMEOUT, tilr_bus::close::TIMEOUT_REASON);
        drop(out);
        if let Ok(closed) = writer.await {
            finish_close(&mut source, socket, closed).await;
        }
    } else if !writer.is_finished() {
        writer.abort();
    }
}

/// Serve an instance's stream socket (`fd`), as its runner attached it.
pub(super) fn spawn_stream(
    broker: BrokerHandle,
    id: ConnId,
    fd: OwnedFd,
    seat: Seat,
    state: watch::Sender<ConnState>,
) {
    tokio::spawn(async move {
        let stream = std::os::unix::net::UnixStream::from(fd);
        let stream = match stream
            .set_nonblocking(true)
            .and_then(|()| tokio::net::UnixStream::from_std(stream))
        {
            Ok(stream) => stream,
            Err(err) => {
                tracing::warn!(target: "tilrd::bus", connection = id, "instance stream: {err}");
                ConnState::end(&state, err.to_string(), None);
                return;
            }
        };
        let (mut reader, mut writer) = stream.into_split();
        let (out, outbox, mut closing) = out(id, Framing::Stream);
        let mut outbox = WriterQueue::new(outbox, &out);
        let drained = Arc::clone(&out.drained);
        let written = Written::new(&out, &broker);
        let mut write = tokio::spawn(async move {
            loop {
                let outgoing = tokio::select! {
                    biased;
                    how = closing.recv() => {
                        if matches!(how, Some(Closing::Flush)) {
                            while let Some(Outgoing { packet, .. }) = outbox.try_recv() {
                                if writer.write_all(&packet).await.is_err() {
                                    break;
                                }
                            }
                        }
                        break;
                    }
                    packet = outbox.recv(true) => match packet {
                        Some(outgoing) => outgoing,
                        None => break,
                    },
                };
                if writer.write_all(&outgoing.packet).await.is_err() {
                    break;
                }
                written.wrote(outgoing.bulk(), outgoing.packet.len());
                if outbox.written(outgoing) {
                    drained.notify_waiters();
                }
            }
            let _ = writer.shutdown().await;
        });
        let peer = seat.peer;
        let transport = seat.transport;
        let mut seat = Some(seat);
        let mut greeted = false;
        let mut buf = BytesMut::with_capacity(64 << 10);
        let why = loop {
            let envelope = match codec::decode_stream(&mut buf) {
                Ok(Some(envelope)) => envelope,
                Ok(None) => {
                    let read = tokio::select! {
                        biased;
                        _ = &mut write => break "closed by the broker".to_owned(),
                        read = reader.read_buf(&mut buf) => read,
                    };
                    match read {
                        Ok(0) => break "the instance closed its end".to_owned(),
                        Ok(_) => continue,
                        Err(err) => break err.to_string(),
                    }
                }
                // The stream's boundaries are lost: the connection ends.
                Err(err) => break format!("bad frame: {err}"),
            };
            if greeted {
                broker.on_envelope(id, envelope);
            } else {
                broker.ready().await;
                greeted = greet(
                    &broker,
                    &out,
                    &state,
                    envelope,
                    transport,
                    peer,
                    "",
                    seat.take(),
                )
                .await;
                if !greeted {
                    // Let the ConnectionError go out.
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, &mut write).await;
                    break "refused".to_owned();
                }
            }
        };
        broker.disconnect(id);
        tracing::info!(target: "tilrd::bus", connection = id, %peer, "instance stream ended: {why}");
        ConnState::end(&state, why, None);
        out.close(0, "");
        drop(out);
        if !write.is_finished() {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, write).await;
        }
    });
}
