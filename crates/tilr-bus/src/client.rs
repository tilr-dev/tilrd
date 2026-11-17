//! The async client of the bus (design B4-B7, K2-K3; the frozen surface of the
//! `bus-wire` change).
//!
//! ```no_run
//! # async fn demo() -> Result<(), tilr_bus::BusError> {
//! use tilr_bus::{Bus, CallOptions, ConnectOptions, Transport};
//! let bus = Bus::connect(Transport::from_env()?, ConnectOptions::new("demo")).await?;
//! let reply = bus.call("test.echo", "hi", CallOptions::default()).await?;
//! assert_eq!(reply.payload.as_ref(), b"hi");
//! # Ok(()) }
//! ```
//!
//! A connection has a reader task, which dispatches what arrives, and a writer
//! task, which writes what the participant sends in the order sent. Calls wait for
//! a slot when `HelloAck.max_in_flight` of them are pending; sends wait while 4 MiB
//! are queued for the socket. A provided method's handler runs as its own task per
//! call, so a handler awaiting a nested call does not hold up others; a `Cancel`
//! aborts it without a reply.

use std::collections::HashMap;
use std::future::Future;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use base64::Engine;
use bytes::{Bytes, BytesMut};
use futures_util::{FutureExt, Sink, SinkExt, Stream, StreamExt};
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::{
    self, Message as WsMessage,
    client::IntoClientRequest,
    http::HeaderValue,
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};

use crate::bus::{self as wire, Envelope, Kind, Lane, MethodSpec, Select, envelope::Msg};
use crate::codec::{self, CodecError};
use crate::{PROTOCOL_VERSION, SUBPROTOCOL, codes, env, limits, names};

/// Bytes of envelopes a participant may have queued for its socket before sends wait.
const SEND_BUDGET: usize = 4 << 20;
/// Events held for a subscription that is not read; more are dropped (events are
/// lossy by design, B6).
const SUBSCRIPTION_QUEUE: usize = 1024;
/// How long the broker has to answer `Hello` by default.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The URL a WebSocket handshake on a Unix socket asks for (the server serves the
/// bus on every path).
const UNIX_URL: &str = "ws://localhost/";

// ---------------------------------------------------------------------------
// Errors, credentials, payload types
// ---------------------------------------------------------------------------

/// A failure with a code of [`codes`] and a message meant for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusError {
    pub code: String,
    pub message: String,
}

impl BusError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(codes::UNAVAILABLE, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(codes::BAD_REQUEST, message)
    }

    pub fn bad_reply(message: impl Into<String>) -> Self {
        Self::new(codes::BAD_REPLY, message)
    }

    pub fn handler_error(message: impl Into<String>) -> Self {
        Self::new(codes::HANDLER_ERROR, message)
    }

    pub fn code(&self) -> &str {
        &self.code
    }
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BusError {}

impl From<wire::Error> for BusError {
    fn from(error: wire::Error) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<BusError> for wire::Error {
    fn from(error: BusError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

/// A token of the server, carried as opaque bytes (`Hello.credential`,
/// `Call.credential`, `HelloAck.credential`, `CredentialUpdate`). Its bytes are
/// never shown by `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential(Bytes);

impl Credential {
    pub fn from_bytes(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &Bytes {
        &self.0
    }

    /// The token as `tilr bus token` prints it: base64url without padding.
    pub fn to_base64url(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.0)
    }

    /// Read a token printed by `tilr bus token` (padding and surrounding space allowed).
    pub fn from_base64url(text: &str) -> Result<Self, BusError> {
        let text = text.trim().trim_end_matches('=');
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(text)
            .map(|bytes| Self(Bytes::from(bytes)))
            .map_err(|err| BusError::bad_request(format!("credential is not base64url: {err}")))
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Credential({} bytes)", self.0.len())
    }
}

/// The answer to a call, or what a handler answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub payload: Bytes,
    /// Empty means `application/x-protobuf`.
    pub content_type: String,
}

impl Reply {
    pub fn new(payload: impl Into<Bytes>) -> Self {
        Self {
            payload: payload.into(),
            content_type: String::new(),
        }
    }

    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = content_type.into();
        self
    }

    /// A protobuf message as the payload.
    pub fn encode<M: Message>(message: &M) -> Self {
        Self::new(message.encode_to_vec())
    }

    /// The payload as a protobuf message; one that does not decode is `bad_reply`.
    pub fn decode<M: Message + Default>(&self) -> Result<M, BusError> {
        M::decode(self.payload.clone())
            .map_err(|err| BusError::bad_reply(format!("reply does not decode: {err}")))
    }
}

impl From<Bytes> for Reply {
    fn from(payload: Bytes) -> Self {
        Self::new(payload)
    }
}

impl From<Vec<u8>> for Reply {
    fn from(payload: Vec<u8>) -> Self {
        Self::new(payload)
    }
}

impl From<String> for Reply {
    fn from(payload: String) -> Self {
        Self::new(payload)
    }
}

impl From<&'static str> for Reply {
    fn from(payload: &'static str) -> Self {
        Self::new(payload)
    }
}

/// One chunk of a stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chunk {
    pub payload: Bytes,
    /// `zstd` when the producer compressed the payload.
    pub content_encoding: String,
}

impl Chunk {
    /// The payload, decompressed when its producer compressed it.
    pub fn bytes(&self) -> Result<Bytes, BusError> {
        match self.content_encoding.as_str() {
            "" => Ok(self.payload.clone()),
            codec::ENCODING_ZSTD => {
                let decoder = zstd::stream::Decoder::new(self.payload.as_ref())
                    .map_err(|err| BusError::bad_reply(format!("chunk: zstd: {err}")))?;
                let mut plain = Vec::new();
                std::io::Read::read_to_end(
                    &mut std::io::Read::take(decoder, codec::MAX_FRAME as u64 + 1),
                    &mut plain,
                )
                .map_err(|err| BusError::bad_reply(format!("chunk: zstd: {err}")))?;
                if plain.len() > codec::MAX_FRAME {
                    return Err(BusError::new(codes::TOO_LARGE, "chunk is over 16 MiB"));
                }
                Ok(Bytes::from(plain))
            }
            other => Err(BusError::bad_reply(format!(
                "chunk has an unknown content encoding {other:?}"
            ))),
        }
    }

    /// The payload as a protobuf message, decompressed first if need be.
    pub fn decode<M: Message + Default>(&self) -> Result<M, BusError> {
        M::decode(self.bytes()?)
            .map_err(|err| BusError::bad_reply(format!("chunk does not decode: {err}")))
    }
}

/// An event a subscription received.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub subject: String,
    pub payload: Bytes,
    pub content_type: String,
    /// The emitter's identity, set by the broker.
    pub from: String,
    /// `pane:<id>` for the core's pane events; empty otherwise.
    pub scope: String,
}

impl Event {
    pub fn decode<M: Message + Default>(&self) -> Result<M, BusError> {
        M::decode(self.payload.clone())
            .map_err(|err| BusError::bad_reply(format!("event does not decode: {err}")))
    }
}

/// How the connection ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloseInfo {
    /// The WebSocket close code (4000-4003, 1002, ...); `None` on stream sockets or
    /// without a close frame.
    pub code: Option<u16>,
    pub reason: String,
    /// The `ConnectionError` the broker sent, or what broke the connection.
    pub error: Option<BusError>,
}

impl CloseInfo {
    fn pending_error(&self) -> BusError {
        if let Some(error) = &self.error {
            return error.clone();
        }
        match self.code {
            Some(code) => {
                BusError::unavailable(format!("connection closed ({code} {})", self.reason))
            }
            None => BusError::unavailable("connection closed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options of one call.
#[derive(Clone, Debug, Default)]
pub struct CallOptions {
    /// A participant's identity; `None` routes by method.
    pub target: Option<String>,
    /// The call's deadline: sent as `Call.timeout_ms`, and the call is cancelled
    /// locally when it passes. `None`: the broker's default (10 s).
    pub timeout: Option<Duration>,
    /// `None`: `application/x-protobuf`.
    pub content_type: Option<String>,
    /// Authorizes this call alone (R4).
    pub credential: Option<Credential>,
    pub trace_id: Option<String>,
}

impl CallOptions {
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    pub fn credential(mut self, credential: Credential) -> Self {
        self.credential = Some(credential);
        self
    }

    pub fn trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}

/// Options of one stream.
#[derive(Clone, Debug, Default)]
pub struct StreamOptions {
    pub target: Option<String>,
    /// Sent as `Call.timeout_ms`; streams have no default deadline.
    pub timeout: Option<Duration>,
    pub content_type: Option<String>,
    pub credential: Option<Credential>,
    pub trace_id: Option<String>,
    /// The initial credit in payload bytes; 0 = `HelloAck.stream_window` (1 MiB).
    pub window: u32,
}

impl StreamOptions {
    pub fn window(mut self, window: u32) -> Self {
        self.window = window;
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    pub fn credential(mut self, credential: Credential) -> Self {
        self.credential = Some(credential);
        self
    }

    pub fn trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}

fn timeout_ms(timeout: Option<Duration>) -> u32 {
    match timeout {
        None => 0,
        Some(timeout) => timeout.as_millis().clamp(1, u32::MAX as u128) as u32,
    }
}

impl MethodSpec {
    /// A method answered with one reply, on the interactive lane.
    pub fn unary(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            kind: Kind::Unary as i32,
            lane: Lane::Interactive as i32,
            select: Select::RoundRobin as i32,
        }
    }

    /// A method answered with a stream of chunks on `lane`.
    pub fn stream(method: impl Into<String>, lane: Lane) -> Self {
        Self {
            method: method.into(),
            kind: Kind::Stream as i32,
            lane: lane as i32,
            select: Select::RoundRobin as i32,
        }
    }

    /// How a call without a target picks among several providers (B11).
    pub fn with_select(mut self, select: Select) -> Self {
        self.select = select as i32;
        self
    }

    /// Answers go on `lane` of the consumer's connection.
    pub fn with_lane(mut self, lane: Lane) -> Self {
        self.lane = lane as i32;
        self
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type UnaryFn = dyn Fn(Request) -> BoxFuture<Result<Reply, BusError>> + Send + Sync;
type StreamFn =
    dyn Fn(Request, StreamWriter) -> BoxFuture<Result<Option<Bytes>, BusError>> + Send + Sync;

/// What runs for a provided method. A handler's `Err` is the call's error reply; a
/// panic is answered `handler_error`.
#[derive(Clone)]
pub enum Handler {
    /// Answers with one [`Reply`].
    Unary(Arc<UnaryFn>),
    /// Writes chunks with the [`StreamWriter`]; `Ok(payload)` ends the stream
    /// with `End{payload}`.
    Stream(Arc<StreamFn>),
}

impl Handler {
    pub fn unary<F, Fut, R>(handler: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<R, BusError>> + Send + 'static,
        R: Into<Reply>,
    {
        Self::Unary(Arc::new(move |request| {
            let reply = handler(request);
            Box::pin(async move { reply.await.map(Into::into) })
        }))
    }

    /// A unary handler of protobuf messages: a request that does not decode is
    /// answered `bad_request`.
    pub fn typed<Req, Resp, F, Fut>(handler: F) -> Self
    where
        Req: Message + Default + 'static,
        Resp: Message + 'static,
        F: Fn(Request, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, BusError>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        Self::Unary(Arc::new(move |request: Request| {
            let handler = handler.clone();
            Box::pin(async move {
                let decoded = request.decode::<Req>()?;
                let reply = handler(request, decoded).await?;
                Ok(Reply::encode(&reply))
            })
        }))
    }

    pub fn stream<F, Fut>(handler: F) -> Self
    where
        F: Fn(Request, StreamWriter) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<Bytes>, BusError>> + Send + 'static,
    {
        Self::Stream(Arc::new(move |request, writer| {
            Box::pin(handler(request, writer))
        }))
    }

    fn kind(&self) -> Kind {
        match self {
            Self::Unary(_) => Kind::Unary,
            Self::Stream(_) => Kind::Stream,
        }
    }
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handler::{:?}", self.kind())
    }
}

/// A call to a provided method, as its handler sees it.
pub struct Request {
    /// The call's id on this connection.
    pub id: u64,
    pub method: String,
    /// The caller's identity, set by the broker.
    pub from: String,
    pub payload: Bytes,
    pub content_type: String,
    pub trace_id: String,
    /// `Call.timeout_ms` as received: the time the caller had left (0 = none).
    pub timeout_ms: u32,
    /// When the call's time is up, from `timeout_ms`.
    pub deadline: Option<Instant>,
    /// The per-call credential the caller presented (empty = none).
    pub credential: Bytes,
    /// For a stream: the caller's initial window.
    pub window: u32,
    bus: Bus,
}

impl Request {
    /// The connection the call came on, for nested calls.
    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    /// The payload as a protobuf message; one that does not decode is `bad_request`.
    pub fn decode<M: Message + Default>(&self) -> Result<M, BusError> {
        M::decode(self.payload.clone()).map_err(|err| {
            BusError::bad_request(format!("{}: request does not decode: {err}", self.method))
        })
    }

    /// The time left until the deadline, for a nested call's timeout.
    pub fn time_left(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("id", &self.id)
            .field("method", &self.method)
            .field("from", &self.from)
            .field("payload_len", &self.payload.len())
            .field("content_type", &self.content_type)
            .field("timeout_ms", &self.timeout_ms)
            .field("credential", &!self.credential.is_empty())
            .finish()
    }
}

/// Credit a provider holds for one stream (B7).
struct CreditGate {
    state: Mutex<(u64, bool)>,
    notify: Notify,
}

impl CreditGate {
    fn new(initial: u64) -> Self {
        Self {
            state: Mutex::new((initial, false)),
            notify: Notify::new(),
        }
    }

    fn add(&self, bytes: u64) {
        lock(&self.state).0 += bytes;
        self.notify.notify_waiters();
    }

    fn close(&self) {
        lock(&self.state).1 = true;
        self.notify.notify_waiters();
    }

    fn available(&self) -> u64 {
        lock(&self.state).0
    }

    fn try_take(&self, bytes: u64) -> Result<bool, BusError> {
        let mut state = lock(&self.state);
        if state.1 {
            return Err(BusError::new(codes::CANCELLED, "the stream has ended"));
        }
        if state.0 >= bytes {
            state.0 -= bytes;
            return Ok(true);
        }
        Ok(false)
    }

    async fn take(&self, bytes: u64) -> Result<(), BusError> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.try_take(bytes)? {
                return Ok(());
            }
            notified.await;
        }
    }
}

/// Writes the chunks of a provided stream, within the credit its reader gave.
pub struct StreamWriter {
    bus: Bus,
    id: u64,
    gate: Arc<CreditGate>,
    window: u32,
}

impl StreamWriter {
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The reader's initial window; no chunk may be larger.
    pub fn window(&self) -> u32 {
        self.window
    }

    /// Payload bytes this provider may send now.
    pub fn credit(&self) -> u64 {
        self.gate.available()
    }

    /// Send a chunk once the reader's credit allows it.
    pub async fn write(&self, payload: impl Into<Bytes>) -> Result<(), BusError> {
        self.write_encoded(payload, "").await
    }

    /// Send a chunk whose payload is already encoded (`zstd`), once credit allows.
    pub async fn write_encoded(
        &self,
        payload: impl Into<Bytes>,
        content_encoding: &str,
    ) -> Result<(), BusError> {
        let payload = payload.into();
        self.check_size(&payload)?;
        self.gate.take(payload.len() as u64).await?;
        self.send(payload, content_encoding).await
    }

    /// Send a chunk only if the credit allows it now: `Ok(false)` when it does not
    /// (a latest-wins provider skips it).
    pub async fn try_write(
        &self,
        payload: impl Into<Bytes>,
        content_encoding: &str,
    ) -> Result<bool, BusError> {
        let payload = payload.into();
        self.check_size(&payload)?;
        if !self.gate.try_take(payload.len() as u64)? {
            return Ok(false);
        }
        self.send(payload, content_encoding).await?;
        Ok(true)
    }

    fn check_size(&self, payload: &Bytes) -> Result<(), BusError> {
        if payload.len() as u64 > u64::from(self.window) {
            return Err(BusError::new(
                codes::TOO_LARGE,
                format!(
                    "a chunk of {} bytes is larger than the stream's window of {}",
                    payload.len(),
                    self.window
                ),
            ));
        }
        Ok(())
    }

    async fn send(&self, payload: Bytes, content_encoding: &str) -> Result<(), BusError> {
        self.bus
            .inner()
            .send(Msg::Chunk(wire::Chunk {
                id: self.id,
                payload,
                content_encoding: content_encoding.to_owned(),
            }))
            .await
    }
}

// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

/// How to reach the broker. Its `Debug` hides the tokens.
pub enum Transport {
    /// An inherited stream socket (an instance's descriptor 3, `TILR_BUS_FD`).
    Fd(OwnedFd),
    /// An instance socket (`TILR_BUS_SOCK`) with its one-time token
    /// (`TILR_BUS_TOKEN`, sent as `Hello.instance_token`).
    Socket { path: PathBuf, token: String },
    /// A WebSocket URL (`ws://` or `wss://`) with subprotocol `tilr.bus.v1`: the
    /// owner with the server's `access_token`, or an outside participant with a
    /// `credential`; `origin` sets the `Origin` header.
    WebSocket {
        url: String,
        access_token: Option<String>,
        credential: Option<Credential>,
        origin: Option<String>,
    },
    /// A WebSocket on the server's Unix socket: the owner (peer uid), or a
    /// participant with a `credential`.
    UnixWebSocket {
        path: PathBuf,
        credential: Option<Credential>,
        origin: Option<String>,
    },
}

impl Transport {
    /// The transport the environment names, in the SDKs' order (K2):
    /// `TILR_BUS_FD`; else `TILR_BUS_SOCK` with `TILR_BUS_TOKEN`; else `TILR_BUS_URL`
    /// with `TILR_BUS_CREDENTIAL` (optional), where a URL `unix:<path>` names the
    /// server's Unix socket. With none: `unavailable` naming the variables.
    ///
    /// `TILR_BUS_FD` hands this process's descriptor over to the transport. It must
    /// be an open socket, and it is taken once: a second call that would take it
    /// again is `bad_request`.
    pub fn from_env() -> Result<Self, BusError> {
        static BUS_FD_TAKEN: AtomicBool = AtomicBool::new(false);
        Self::from_lookup(|name| std::env::var(name).ok(), &BUS_FD_TAKEN)
    }

    /// [`Transport::from_env`] with the variables read by `lookup`; `taken` says
    /// whether the descriptor was taken already.
    pub(crate) fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        taken: &AtomicBool,
    ) -> Result<Self, BusError> {
        let get = |name: &str| lookup(name).filter(|value| !value.is_empty());
        if let Some(fd) = get(env::BUS_FD) {
            let fd: i32 = fd.parse().ok().filter(|fd| *fd >= 0).ok_or_else(|| {
                BusError::bad_request(format!("{}={fd} is not a descriptor", env::BUS_FD))
            })?;
            return take_bus_fd(fd, taken).map(Self::Fd);
        }
        if let Some(path) = get(env::BUS_SOCK) {
            let token = get(env::BUS_TOKEN).ok_or_else(|| {
                BusError::unavailable(format!(
                    "{} is set without {}",
                    env::BUS_SOCK,
                    env::BUS_TOKEN
                ))
            })?;
            return Ok(Self::Socket {
                path: PathBuf::from(path),
                token,
            });
        }
        if let Some(url) = get(env::BUS_URL) {
            let credential = get(env::BUS_CREDENTIAL)
                .map(|text| Credential::from_base64url(&text))
                .transpose()?;
            if let Some(path) = url.strip_prefix("unix:") {
                return Ok(Self::UnixWebSocket {
                    path: PathBuf::from(path),
                    credential,
                    origin: None,
                });
            }
            return Ok(Self::WebSocket {
                url,
                access_token: None,
                credential,
                origin: None,
            });
        }
        Err(BusError::unavailable(format!(
            "no bus to connect to: set {}, {} and {}, or {} (and {})",
            env::BUS_FD,
            env::BUS_SOCK,
            env::BUS_TOKEN,
            env::BUS_URL,
            env::BUS_CREDENTIAL
        )))
    }
}

/// Take ownership of the inherited bus descriptor `fd`, once, if it is an open
/// socket: never a descriptor the process uses otherwise (stdout, a file).
fn take_bus_fd(fd: i32, taken: &AtomicBool) -> Result<OwnedFd, BusError> {
    let refuse = |what: &str| BusError::bad_request(format!("{}={fd} {what}", env::BUS_FD));
    if taken.load(Ordering::SeqCst) {
        return Err(refuse("was taken already"));
    }
    // SAFETY: F_GETFD only reads the descriptor flags of `fd`; an fd that is not
    // open is answered -1 (EBADF).
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
        return Err(refuse("is not an open descriptor"));
    }
    // SAFETY: `stat` is a plain struct filled in by fstat; it is read only on success.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fstat writes into `stat`, which lives for the call.
    if unsafe { libc::fstat(fd, &mut stat) } == -1 || stat.st_mode & libc::S_IFMT != libc::S_IFSOCK
    {
        return Err(refuse("is not a socket"));
    }
    if taken.swap(true, Ordering::SeqCst) {
        return Err(refuse("was taken already"));
    }
    // SAFETY: `fd` is an open socket the server left for this process to own
    // (design B10), and the flag above lets it be taken once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl std::fmt::Debug for Transport {
    /// Tokens are never shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secret = |set: bool| if set { "<redacted>" } else { "<none>" };
        match self {
            Self::Fd(fd) => f.debug_tuple("Fd").field(&fd.as_raw_fd()).finish(),
            Self::Socket { path, token } => f
                .debug_struct("Socket")
                .field("path", path)
                .field("token", &secret(!token.is_empty()))
                .finish(),
            Self::WebSocket {
                url,
                access_token,
                credential,
                origin,
            } => f
                .debug_struct("WebSocket")
                .field("url", url)
                .field("access_token", &secret(access_token.is_some()))
                .field("credential", credential)
                .field("origin", origin)
                .finish(),
            Self::UnixWebSocket {
                path,
                credential,
                origin,
            } => f
                .debug_struct("UnixWebSocket")
                .field("path", path)
                .field("credential", credential)
                .field("origin", origin)
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Framing {
    WebSocket,
    Stream,
}

impl Framing {
    fn encode(self, envelope: &Envelope) -> Result<Bytes, CodecError> {
        match self {
            Self::WebSocket => codec::encode_ws(envelope),
            Self::Stream => codec::encode_stream(envelope),
        }
    }
}

type WsSink = Pin<Box<dyn Sink<WsMessage, Error = tungstenite::Error> + Send>>;
type WsSource = Pin<Box<dyn Stream<Item = Result<WsMessage, tungstenite::Error>> + Send>>;

enum FrameWriter {
    Stream(Box<dyn AsyncWrite + Send + Unpin>),
    Ws(WsSink),
}

impl FrameWriter {
    async fn write(&mut self, frame: Bytes) -> Result<(), String> {
        match self {
            Self::Stream(write) => write.write_all(&frame).await.map_err(|err| err.to_string()),
            Self::Ws(sink) => sink
                .send(WsMessage::Binary(frame))
                .await
                .map_err(|err| err.to_string()),
        }
    }

    async fn close(&mut self) {
        match self {
            Self::Stream(write) => {
                let _ = write.shutdown().await;
            }
            Self::Ws(sink) => {
                let _ = sink
                    .send(WsMessage::Close(Some(CloseFrame {
                        code: CloseCode::Normal,
                        reason: "".into(),
                    })))
                    .await;
                let _ = sink.close().await;
            }
        }
    }
}

enum FrameReader {
    Stream {
        read: Box<dyn AsyncRead + Send + Unpin>,
        buf: BytesMut,
    },
    Ws(WsSource),
}

enum Incoming {
    Envelope(Envelope),
    /// A WebSocket message that did not decode; the connection stays.
    Bad(String),
    Closed(CloseInfo),
}

impl FrameReader {
    async fn recv(&mut self) -> Incoming {
        match self {
            Self::Stream { read, buf } => loop {
                match codec::decode_stream(buf) {
                    Ok(Some(envelope)) => return Incoming::Envelope(envelope),
                    Ok(None) => {}
                    Err(err) => {
                        return Incoming::Closed(CloseInfo {
                            error: Some(BusError::new(
                                codes::BAD_FRAME,
                                format!("bad packet: {err}"),
                            )),
                            ..CloseInfo::default()
                        });
                    }
                }
                if buf.capacity() - buf.len() < 8192 {
                    buf.reserve(64 << 10);
                }
                match read.read_buf(buf).await {
                    Ok(0) => return Incoming::Closed(CloseInfo::default()),
                    Ok(_) => {}
                    Err(err) => {
                        return Incoming::Closed(CloseInfo {
                            error: Some(BusError::unavailable(format!("read: {err}"))),
                            ..CloseInfo::default()
                        });
                    }
                }
            },
            Self::Ws(source) => loop {
                match source.next().await {
                    Some(Ok(WsMessage::Binary(message))) => {
                        return match codec::decode_ws(&message) {
                            Ok(envelope) => Incoming::Envelope(envelope),
                            Err(err) => Incoming::Bad(format!("bad packet: {err}")),
                        };
                    }
                    Some(Ok(WsMessage::Text(_))) => {
                        return Incoming::Bad("bad packet: a text message".into());
                    }
                    Some(Ok(WsMessage::Close(frame))) => {
                        return Incoming::Closed(CloseInfo {
                            code: frame.as_ref().map(|frame| u16::from(frame.code)),
                            reason: frame
                                .map(|frame| frame.reason.to_string())
                                .unwrap_or_default(),
                            error: None,
                        });
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        return Incoming::Closed(CloseInfo {
                            error: Some(BusError::unavailable(format!("websocket: {err}"))),
                            ..CloseInfo::default()
                        });
                    }
                    None => return Incoming::Closed(CloseInfo::default()),
                }
            },
        }
    }
}

/// What `Hello` carries for the transport.
#[derive(Default)]
struct HelloAuth {
    access_token: String,
    credential: Option<Credential>,
    instance_token: String,
}

fn ws_config() -> WebSocketConfig {
    // A message is a flag byte and an envelope of up to 16 MiB, compressed or not
    // (zstd's worst case adds a little); the codec enforces the exact limit.
    let max = codec::MAX_FRAME + (1 << 20);
    WebSocketConfig::default()
        .max_message_size(Some(max))
        .max_frame_size(Some(max))
}

fn handshake_error(target: &str, err: tungstenite::Error) -> BusError {
    if let tungstenite::Error::Http(response) = &err {
        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return BusError::new(codes::DENIED, format!("{target}: HTTP {status}"));
        }
    }
    BusError::unavailable(format!("{target}: {err}"))
}

fn ws_request(
    url: &str,
    origin: Option<&str>,
) -> Result<tungstenite::handshake::client::Request, BusError> {
    let mut request = url
        .into_client_request()
        .map_err(|err| BusError::bad_request(format!("{url}: {err}")))?;
    request.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_static(SUBPROTOCOL),
    );
    if let Some(origin) = origin {
        let value = HeaderValue::from_str(origin)
            .map_err(|err| BusError::bad_request(format!("origin {origin:?}: {err}")))?;
        request.headers_mut().insert("origin", value);
    }
    Ok(request)
}

fn split_ws<S>(ws: tokio_tungstenite::WebSocketStream<S>) -> (FrameReader, FrameWriter)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sink, source) = ws.split();
    (
        FrameReader::Ws(Box::pin(source)),
        FrameWriter::Ws(Box::pin(sink)),
    )
}

fn split_stream(stream: tokio::net::UnixStream) -> (FrameReader, FrameWriter) {
    let (read, write) = stream.into_split();
    (
        FrameReader::Stream {
            read: Box::new(read),
            buf: BytesMut::with_capacity(64 << 10),
        },
        FrameWriter::Stream(Box::new(write)),
    )
}

async fn open(
    transport: Transport,
) -> Result<(FrameReader, FrameWriter, Framing, HelloAuth), BusError> {
    match transport {
        Transport::Fd(fd) => {
            let stream = std::os::unix::net::UnixStream::from(fd);
            stream
                .set_nonblocking(true)
                .map_err(|err| BusError::unavailable(format!("bus descriptor: {err}")))?;
            let stream = tokio::net::UnixStream::from_std(stream)
                .map_err(|err| BusError::unavailable(format!("bus descriptor: {err}")))?;
            let (reader, writer) = split_stream(stream);
            Ok((reader, writer, Framing::Stream, HelloAuth::default()))
        }
        Transport::Socket { path, token } => {
            let stream = tokio::net::UnixStream::connect(&path)
                .await
                .map_err(|err| {
                    BusError::unavailable(format!("connect to {}: {err}", path.display()))
                })?;
            let (reader, writer) = split_stream(stream);
            let auth = HelloAuth {
                instance_token: token,
                ..HelloAuth::default()
            };
            Ok((reader, writer, Framing::Stream, auth))
        }
        Transport::WebSocket {
            url,
            access_token,
            credential,
            origin,
        } => {
            let request = ws_request(&url, origin.as_deref())?;
            let (reader, writer) = if url.starts_with("wss://") {
                let (ws, _) = tilr_websocket::connect_with_config(request, Some(ws_config()))
                    .await
                    .map_err(|err| handshake_error(&url, err))?;
                split_ws(ws)
            } else {
                let (ws, _) =
                    tokio_tungstenite::connect_async_with_config(request, Some(ws_config()), false)
                        .await
                        .map_err(|err| handshake_error(&url, err))?;
                split_ws(ws)
            };
            let auth = HelloAuth {
                access_token: access_token.unwrap_or_default(),
                credential,
                ..HelloAuth::default()
            };
            Ok((reader, writer, Framing::WebSocket, auth))
        }
        Transport::UnixWebSocket {
            path,
            credential,
            origin,
        } => {
            let target = path.display().to_string();
            let stream = tokio::net::UnixStream::connect(&path)
                .await
                .map_err(|err| BusError::unavailable(format!("connect to {target}: {err}")))?;
            let request = ws_request(UNIX_URL, origin.as_deref())?;
            let (ws, _) =
                tokio_tungstenite::client_async_with_config(request, stream, Some(ws_config()))
                    .await
                    .map_err(|err| handshake_error(&target, err))?;
            let (reader, writer) = split_ws(ws);
            let auth = HelloAuth {
                credential,
                ..HelloAuth::default()
            };
            Ok((reader, writer, Framing::WebSocket, auth))
        }
    }
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/// What a participant says in its `Hello`, including the methods it provides
/// (there is no later registration, B4).
#[derive(Clone, Debug)]
pub struct ConnectOptions {
    pub client_name: String,
    /// Names this participant for as long as it runs (a UUID); empty = none.
    pub client_id: String,
    pub handshake_timeout: Duration,
    provides: Vec<(MethodSpec, Handler)>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self::new("")
    }
}

impl ConnectOptions {
    pub fn new(client_name: impl Into<String>) -> Self {
        Self {
            client_name: client_name.into(),
            client_id: String::new(),
            handshake_timeout: HANDSHAKE_TIMEOUT,
            provides: Vec::new(),
        }
    }

    pub fn client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = client_id.into();
        self
    }

    /// How long connecting may take in all: the connection, TLS, the WebSocket
    /// upgrade, `Hello` and `HelloAck` (default 10 s); past it, `timeout`.
    pub fn handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Provide `spec.method`, answered by `handler`; its kind is the handler's.
    pub fn provide(mut self, mut spec: MethodSpec, handler: Handler) -> Self {
        spec.kind = handler.kind() as i32;
        self.provides.push((spec, handler));
        self
    }
}

/// A connection to the bus. Cheap to clone; the connection ends when the last
/// clone (and every stream, subscription and running handler holding one) is gone,
/// or on [`Bus::close`].
#[derive(Clone)]
pub struct Bus(Arc<Handle>);

struct Handle {
    inner: Arc<Inner>,
    reader: AbortHandle,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.reader.abort();
        self.inner.shutdown(CloseInfo {
            reason: "closed by the client".into(),
            ..CloseInfo::default()
        });
    }
}

enum Outgoing {
    Frame(Bytes, Option<OwnedSemaphorePermit>),
    Close,
}

struct CallerStream {
    tx: mpsc::UnboundedSender<Result<StreamItem, BusError>>,
    /// Credit given so far (window plus credits) and payload bytes received.
    allowed: u64,
    received: u64,
}

enum StreamItem {
    Chunk(Chunk),
    End(Bytes),
}

struct Served {
    abort: Option<AbortHandle>,
    gate: Option<Arc<CreditGate>>,
}

struct SubscriptionQueue {
    pattern: String,
    tx: mpsc::Sender<Event>,
    /// Events dropped because the queue was full.
    dropped: Arc<AtomicU64>,
}

#[derive(Default)]
struct State {
    next_id: u64,
    calls: HashMap<u64, oneshot::Sender<Result<Reply, BusError>>>,
    streams: HashMap<u64, CallerStream>,
    subscriptions: HashMap<u64, SubscriptionQueue>,
    served: HashMap<u64, Served>,
    closed: bool,
}

struct Inner {
    framing: Framing,
    ack: wire::HelloAck,
    max_frame: usize,
    out: mpsc::UnboundedSender<Outgoing>,
    budget: Arc<Semaphore>,
    in_flight: Arc<Semaphore>,
    handlers: HashMap<String, Handler>,
    state: Mutex<State>,
    credential: Mutex<Option<Credential>>,
    closed: watch::Sender<Option<CloseInfo>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn envelope(msg: Msg) -> Envelope {
    Envelope { msg: Some(msg) }
}

impl Inner {
    fn encode(&self, msg: Msg) -> Result<Bytes, BusError> {
        let envelope = envelope(msg);
        let len = envelope.encoded_len();
        if len > self.max_frame {
            return Err(BusError::new(
                codes::TOO_LARGE,
                format!(
                    "an envelope of {len} bytes is over the limit of {}",
                    self.max_frame
                ),
            ));
        }
        self.framing
            .encode(&envelope)
            .map_err(|err| BusError::new(codes::TOO_LARGE, err.to_string()))
    }

    /// Queue an envelope, waiting while the send budget is spent.
    async fn send(&self, msg: Msg) -> Result<(), BusError> {
        let frame = self.encode(msg)?;
        let permits = frame.len().clamp(1, SEND_BUDGET) as u32;
        let permit = self
            .budget
            .clone()
            .acquire_many_owned(permits)
            .await
            .map_err(|_| self.closed_error())?;
        self.out
            .send(Outgoing::Frame(frame, Some(permit)))
            .map_err(|_| self.closed_error())
    }

    /// Queue a small control envelope at once (credits, cancels, unsubscribes).
    fn send_now(&self, msg: Msg) {
        if let Ok(frame) = self.encode(msg) {
            let _ = self.out.send(Outgoing::Frame(frame, None));
        }
    }

    fn closed_error(&self) -> BusError {
        match &*self.closed.borrow() {
            Some(info) => info.pending_error(),
            None => BusError::unavailable("connection closed"),
        }
    }

    fn next_id(state: &mut State) -> u64 {
        state.next_id += 1;
        state.next_id
    }

    fn shutdown(&self, info: CloseInfo) {
        let mut state = lock(&self.state);
        if state.closed {
            return;
        }
        state.closed = true;
        let calls = std::mem::take(&mut state.calls);
        let streams = std::mem::take(&mut state.streams);
        let subscriptions = std::mem::take(&mut state.subscriptions);
        let served = std::mem::take(&mut state.served);
        drop(state);
        let error = info.pending_error();
        self.closed.send_replace(Some(info));
        for (_, reply) in calls {
            let _ = reply.send(Err(error.clone()));
        }
        for (_, stream) in streams {
            let _ = stream.tx.send(Err(error.clone()));
        }
        drop(subscriptions);
        for (_, served) in served {
            if let Some(abort) = served.abort {
                abort.abort();
            }
            if let Some(gate) = served.gate {
                gate.close();
            }
        }
        self.in_flight.close();
        self.budget.close();
        let _ = self.out.send(Outgoing::Close);
    }
}

impl Bus {
    /// Connect, say `Hello` and wait for `HelloAck`. A refused `Hello` is the
    /// error the broker sent (`expected_hello`, `unsupported_version`, `denied`,
    /// `reserved`).
    pub async fn connect(transport: Transport, options: ConnectOptions) -> Result<Bus, BusError> {
        for (spec, _) in &options.provides {
            if !names::valid_method(&spec.method) {
                return Err(BusError::bad_request(format!(
                    "{:?} is not a method name",
                    spec.method
                )));
            }
        }
        let handshake = handshake(transport, &options);
        let (reader, writer, framing, auth, ack) =
            match tokio::time::timeout(options.handshake_timeout, handshake).await {
                Ok(result) => result?,
                Err(_) => {
                    return Err(BusError::new(
                        codes::TIMEOUT,
                        format!(
                            "no HelloAck within {} ms of connecting",
                            options.handshake_timeout.as_millis()
                        ),
                    ));
                }
            };
        let max_frame = match ack.max_frame {
            0 => codec::MAX_FRAME,
            max => (max as usize).min(codec::MAX_FRAME),
        };
        let max_in_flight = match ack.max_in_flight {
            0 => limits::MAX_IN_FLIGHT,
            max => max,
        } as usize;
        let credential = if ack.credential.is_empty() {
            auth.credential
        } else {
            Some(Credential::from_bytes(ack.credential.clone()))
        };
        let (out, out_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            framing,
            ack,
            max_frame,
            out,
            budget: Arc::new(Semaphore::new(SEND_BUDGET)),
            in_flight: Arc::new(Semaphore::new(max_in_flight)),
            handlers: options
                .provides
                .into_iter()
                .map(|(spec, handler)| (spec.method, handler))
                .collect(),
            state: Mutex::new(State::default()),
            credential: Mutex::new(credential),
            closed: watch::channel(None).0,
        });
        tokio::spawn(write_loop(writer, out_rx, Arc::downgrade(&inner)));
        // The reader starts once the handle exists: a Call that came in the same
        // read as HelloAck must find a live handle to answer it.
        let (start, started) = oneshot::channel::<()>();
        let bus = Arc::new_cyclic(|weak: &Weak<Handle>| {
            let reader = tokio::spawn({
                let (inner, weak) = (inner.clone(), weak.clone());
                async move {
                    if started.await.is_ok() {
                        read_loop(reader, inner, weak).await;
                    }
                }
            });
            Handle {
                inner: inner.clone(),
                reader: reader.abort_handle(),
            }
        });
        let _ = start.send(());
        Ok(Bus(bus))
    }
}

/// Open the transport, say `Hello` and read `HelloAck`.
async fn handshake(
    transport: Transport,
    options: &ConnectOptions,
) -> Result<(FrameReader, FrameWriter, Framing, HelloAuth, wire::HelloAck), BusError> {
    let (mut reader, mut writer, framing, auth) = open(transport).await?;
    let hello = wire::Hello {
        protocol_version: PROTOCOL_VERSION,
        client_name: options.client_name.clone(),
        client_id: options.client_id.clone(),
        access_token: auth.access_token.clone(),
        credential: auth
            .credential
            .as_ref()
            .map(|credential| credential.as_bytes().clone())
            .unwrap_or_default(),
        instance_token: auth.instance_token.clone(),
        provides: options
            .provides
            .iter()
            .map(|(spec, _)| spec.clone())
            .collect(),
    };
    let frame = framing
        .encode(&envelope(Msg::Hello(hello)))
        .map_err(|err| BusError::new(codes::TOO_LARGE, err.to_string()))?;
    writer
        .write(frame)
        .await
        .map_err(|err| BusError::unavailable(format!("send Hello: {err}")))?;
    let ack = match reader.recv().await {
        Incoming::Envelope(Envelope {
            msg: Some(Msg::HelloAck(ack)),
        }) => ack,
        Incoming::Envelope(Envelope {
            msg: Some(Msg::ConnectionError(error)),
        }) => return Err(BusError::new(error.code, error.message)),
        Incoming::Envelope(_) => {
            return Err(BusError::bad_reply(
                "the broker did not answer Hello with HelloAck",
            ));
        }
        Incoming::Bad(err) => return Err(BusError::new(codes::BAD_FRAME, err)),
        Incoming::Closed(info) => {
            return Err(info.error.clone().unwrap_or_else(|| {
                BusError::unavailable(match info.code {
                    Some(code) => format!("closed before HelloAck ({code} {})", info.reason),
                    None => "closed before HelloAck".into(),
                })
            }));
        }
    };
    if ack.protocol_version != PROTOCOL_VERSION {
        return Err(BusError::new(
            codes::UNSUPPORTED_VERSION,
            format!(
                "unsupported protocol version {}: this client speaks {PROTOCOL_VERSION}",
                ack.protocol_version
            ),
        ));
    }
    Ok((reader, writer, framing, auth, ack))
}

impl Bus {
    fn inner(&self) -> &Inner {
        &self.0.inner
    }

    /// This connection's identity, set by the broker: `owner`, `instance:<name>`,
    /// `token:<name>`.
    pub fn identity(&self) -> &str {
        &self.inner().ack.identity
    }

    pub fn hello_ack(&self) -> &wire::HelloAck {
        &self.inner().ack
    }

    pub fn connection_id(&self) -> u64 {
        self.inner().ack.connection_id
    }

    /// What this connection adds to its input counter for `pane.input.seq`.
    pub fn seq_base(&self) -> u64 {
        self.connection_id() << 32
    }

    /// The token this connection holds: the one it presented, an instance's from
    /// `HelloAck`, or the latest `CredentialUpdate` either way. `None` for the owner.
    pub fn credential(&self) -> Option<Credential> {
        lock(&self.inner().credential).clone()
    }

    /// Present a new token on this connection (after `bus.renew`).
    pub async fn update_credential(&self, credential: Credential) -> Result<(), BusError> {
        self.inner()
            .send(Msg::CredentialUpdate(wire::CredentialUpdate {
                credential: credential.as_bytes().clone(),
            }))
            .await?;
        *lock(&self.inner().credential) = Some(credential);
        Ok(())
    }

    /// Call `method` and wait for its reply. Waits for a slot while
    /// `max_in_flight` calls and streams are pending; dropping the future cancels
    /// the call (`Cancel`).
    pub async fn call(
        &self,
        method: &str,
        payload: impl Into<Bytes>,
        options: CallOptions,
    ) -> Result<Reply, BusError> {
        let inner = self.inner();
        let _permit = inner
            .in_flight
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| inner.closed_error())?;
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut state = lock(&inner.state);
            if state.closed {
                return Err(inner.closed_error());
            }
            let id = Inner::next_id(&mut state);
            state.calls.insert(id, tx);
            id
        };
        let mut pending = PendingCall {
            inner,
            id,
            done: false,
        };
        let sent = inner
            .send(Msg::Call(wire::Call {
                id,
                target: options.target.unwrap_or_default(),
                method: method.to_owned(),
                payload: payload.into(),
                from: String::new(),
                trace_id: options.trace_id.unwrap_or_default(),
                timeout_ms: timeout_ms(options.timeout),
                content_type: options.content_type.unwrap_or_default(),
                credential: options
                    .credential
                    .map(|credential| credential.0)
                    .unwrap_or_default(),
                window: 0,
            }))
            .await;
        if let Err(error) = sent {
            // Never sent: nothing to cancel.
            lock(&inner.state).calls.remove(&id);
            pending.done = true;
            return Err(error);
        }
        let result = match options.timeout {
            None => rx.await,
            Some(timeout) => match tokio::time::timeout(timeout, rx).await {
                Ok(result) => result,
                Err(_) => {
                    return Err(BusError::new(
                        codes::TIMEOUT,
                        format!("{method} did not answer within {} ms", timeout.as_millis()),
                    ));
                }
            },
        };
        pending.done = true;
        result.unwrap_or_else(|_| Err(inner.closed_error()))
    }

    /// Call with a protobuf request and decode the protobuf reply.
    pub async fn call_typed<Req: Message, Resp: Message + Default>(
        &self,
        method: &str,
        request: &Req,
        options: CallOptions,
    ) -> Result<Resp, BusError> {
        self.call(method, request.encode_to_vec(), options)
            .await?
            .decode()
    }

    /// Open a stream: a call answered with chunks. Credit is returned as chunks
    /// are taken; dropping the stream before its end cancels it.
    pub async fn stream(
        &self,
        method: &str,
        payload: impl Into<Bytes>,
        options: StreamOptions,
    ) -> Result<ChunkStream, BusError> {
        let inner = self.inner();
        if options.window > limits::MAX_STREAM_WINDOW {
            return Err(BusError::bad_request(format!(
                "a window of {} is over the limit of {}",
                options.window,
                limits::MAX_STREAM_WINDOW
            )));
        }
        let permit = inner
            .in_flight
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| inner.closed_error())?;
        let window = match options.window {
            0 => match inner.ack.stream_window {
                0 => limits::STREAM_WINDOW,
                window => window,
            },
            window => window,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let id = {
            let mut state = lock(&inner.state);
            if state.closed {
                return Err(inner.closed_error());
            }
            let id = Inner::next_id(&mut state);
            state.streams.insert(
                id,
                CallerStream {
                    tx,
                    allowed: u64::from(window),
                    received: 0,
                },
            );
            id
        };
        let mut stream = ChunkStream {
            bus: self.clone(),
            id,
            rx,
            done: false,
            end: None,
            permit: Some(permit),
        };
        let sent = inner
            .send(Msg::Call(wire::Call {
                id,
                target: options.target.unwrap_or_default(),
                method: method.to_owned(),
                payload: payload.into(),
                from: String::new(),
                trace_id: options.trace_id.unwrap_or_default(),
                timeout_ms: timeout_ms(options.timeout),
                content_type: options.content_type.unwrap_or_default(),
                credential: options
                    .credential
                    .map(|credential| credential.0)
                    .unwrap_or_default(),
                window,
            }))
            .await;
        if let Err(error) = sent {
            // Never sent: nothing to cancel.
            lock(&inner.state).streams.remove(&id);
            stream.done = true;
            return Err(error);
        }
        Ok(stream)
    }

    /// Emit an event (at most once, to the subscribers connected now).
    pub async fn emit(&self, subject: &str, payload: impl Into<Bytes>) -> Result<(), BusError> {
        self.emit_with(subject, payload, "").await
    }

    /// Emit an event with a `content_type`.
    pub async fn emit_with(
        &self,
        subject: &str,
        payload: impl Into<Bytes>,
        content_type: &str,
    ) -> Result<(), BusError> {
        if !names::valid_subject(subject) {
            return Err(BusError::bad_request(format!(
                "{subject:?} is not an event subject"
            )));
        }
        self.inner()
            .send(Msg::Event(wire::Event {
                subject: subject.to_owned(),
                payload: payload.into(),
                content_type: content_type.to_owned(),
                from: String::new(),
                scope: String::new(),
            }))
            .await
    }

    /// Subscribe to the events whose subject matches `pattern`; answered once the
    /// broker took the subscription (`denied` if it is not allowed). Dropping the
    /// subscription ends it.
    pub async fn subscribe(&self, pattern: &str) -> Result<Subscription, BusError> {
        if !names::valid_pattern(pattern) {
            return Err(BusError::bad_request(format!(
                "{pattern:?} is not a subscription pattern"
            )));
        }
        let inner = self.inner();
        let (event_tx, event_rx) = mpsc::channel(SUBSCRIPTION_QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut state = lock(&inner.state);
            if state.closed {
                return Err(inner.closed_error());
            }
            let id = Inner::next_id(&mut state);
            state.subscriptions.insert(
                id,
                SubscriptionQueue {
                    pattern: pattern.to_owned(),
                    tx: event_tx,
                    dropped: dropped.clone(),
                },
            );
            state.calls.insert(id, tx);
            id
        };
        let subscription = Subscription {
            bus: self.clone(),
            id,
            pattern: pattern.to_owned(),
            rx: event_rx,
            dropped,
        };
        inner
            .send(Msg::Subscribe(wire::Subscribe {
                id,
                pattern: pattern.to_owned(),
            }))
            .await?;
        match rx.await {
            Ok(Ok(_)) => Ok(subscription),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(inner.closed_error()),
        }
    }

    /// Tell the broker this participant is now the user's active one for its
    /// `LAST_ACTIVE` methods (B11).
    pub async fn activate(&self) -> Result<(), BusError> {
        self.inner().send(Msg::Activate(wire::Activate {})).await
    }

    pub fn is_closed(&self) -> bool {
        self.inner().closed.borrow().is_some()
    }

    /// Wait until the connection has ended, and say how.
    pub async fn closed(&self) -> CloseInfo {
        let mut rx = self.inner().closed.subscribe();
        loop {
            if let Some(info) = rx.borrow_and_update().clone() {
                return info;
            }
            if rx.changed().await.is_err() {
                return CloseInfo::default();
            }
        }
    }

    /// End the connection: pending calls fail `unavailable`, the socket is closed.
    pub fn close(&self) {
        self.0.reader.abort();
        self.inner().shutdown(CloseInfo {
            reason: "closed by the client".into(),
            ..CloseInfo::default()
        });
    }
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bus")
            .field("identity", &self.identity())
            .field("connection_id", &self.connection_id())
            .finish()
    }
}

/// Removes a call that is given up and tells the broker.
struct PendingCall<'a> {
    inner: &'a Inner,
    id: u64,
    done: bool,
}

impl Drop for PendingCall<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if lock(&self.inner.state).calls.remove(&self.id).is_some() {
            self.inner.send_now(Msg::Cancel(wire::Cancel {
                id: self.id,
                reason: "cancelled by the caller".into(),
            }));
        }
    }
}

/// The chunks of a stream, as they come; `None` after `End`.
pub struct ChunkStream {
    bus: Bus,
    id: u64,
    rx: mpsc::UnboundedReceiver<Result<StreamItem, BusError>>,
    done: bool,
    end: Option<Bytes>,
    /// A slot of `max_in_flight`, given back when the stream ends.
    permit: Option<OwnedSemaphorePermit>,
}

impl ChunkStream {
    /// The stream's call id.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The next chunk; its credit goes back to the provider as it is taken.
    pub async fn next(&mut self) -> Option<Result<Chunk, BusError>> {
        StreamExt::next(self).await
    }

    /// The `End` payload, once the stream ended.
    pub fn end_payload(&self) -> Option<&Bytes> {
        self.end.as_ref()
    }

    /// Stop the stream (`Cancel`); the same as dropping it.
    pub fn cancel(self) {}

    fn finish(&mut self) {
        self.done = true;
        self.permit = None;
    }

    fn credit(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let inner = self.bus.inner();
        if let Some(stream) = lock(&inner.state).streams.get_mut(&self.id) {
            stream.allowed += bytes as u64;
        }
        inner.send_now(Msg::Credit(wire::Credit {
            id: self.id,
            bytes: bytes as u32,
        }));
    }
}

impl Stream for ChunkStream {
    type Item = Result<Chunk, BusError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        match self.rx.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(StreamItem::Chunk(chunk)))) => {
                self.credit(chunk.payload.len());
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Ok(StreamItem::End(payload)))) => {
                self.finish();
                self.end = Some(payload);
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                self.finish();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                self.finish();
                let error = self.bus.inner().closed_error();
                Poll::Ready(Some(Err(error)))
            }
        }
    }
}

impl Drop for ChunkStream {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let inner = self.bus.inner();
        if lock(&inner.state).streams.remove(&self.id).is_some() {
            inner.send_now(Msg::Cancel(wire::Cancel {
                id: self.id,
                reason: "cancelled by the caller".into(),
            }));
        }
    }
}

/// The events of one subscription; `None` once the connection ended. Events that
/// arrive while 1024 are unread are dropped and counted ([`Subscription::dropped`]).
pub struct Subscription {
    bus: Bus,
    id: u64,
    pattern: String,
    rx: mpsc::Receiver<Event>,
    dropped: Arc<AtomicU64>,
}

impl Subscription {
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// How many events were dropped so far because 1024 were unread.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

impl Stream for Subscription {
    type Item = Event;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Event>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let inner = self.bus.inner();
        if lock(&inner.state).subscriptions.remove(&self.id).is_some() {
            inner.send_now(Msg::Unsubscribe(wire::Unsubscribe { id: self.id }));
        }
    }
}

// ---------------------------------------------------------------------------
// Reader and writer tasks
// ---------------------------------------------------------------------------

async fn write_loop(
    mut writer: FrameWriter,
    mut rx: mpsc::UnboundedReceiver<Outgoing>,
    inner: Weak<Inner>,
) {
    while let Some(item) = rx.recv().await {
        match item {
            Outgoing::Frame(frame, permit) => {
                let result = writer.write(frame).await;
                drop(permit);
                if let Err(err) = result {
                    if let Some(inner) = inner.upgrade() {
                        inner.shutdown(CloseInfo {
                            error: Some(BusError::unavailable(format!("write: {err}"))),
                            ..CloseInfo::default()
                        });
                    }
                    return;
                }
            }
            Outgoing::Close => break,
        }
    }
    writer.close().await;
}

async fn read_loop(mut reader: FrameReader, inner: Arc<Inner>, handle: Weak<Handle>) {
    loop {
        match reader.recv().await {
            Incoming::Envelope(envelope) => dispatch(&inner, &handle, envelope),
            // The broker's packets keep their boundaries on a WebSocket: skip it.
            Incoming::Bad(_) => {}
            Incoming::Closed(info) => {
                inner.shutdown(info);
                return;
            }
        }
    }
}

fn dispatch(inner: &Arc<Inner>, handle: &Weak<Handle>, envelope: Envelope) {
    let Some(msg) = envelope.msg else {
        return; // A variant of a later version.
    };
    match msg {
        Msg::Reply(reply) => on_reply(inner, reply),
        Msg::Chunk(chunk) => on_chunk(inner, chunk),
        Msg::End(end) => {
            if let Some(stream) = lock(&inner.state).streams.remove(&end.id) {
                let _ = stream.tx.send(Ok(StreamItem::End(end.payload)));
            }
        }
        Msg::Event(event) => {
            let state = lock(&inner.state);
            for queue in state.subscriptions.values() {
                if !names::matches(&queue.pattern, &event.subject) {
                    continue;
                }
                let sent = queue.tx.try_send(Event {
                    subject: event.subject.clone(),
                    payload: event.payload.clone(),
                    content_type: event.content_type.clone(),
                    from: event.from.clone(),
                    scope: event.scope.clone(),
                });
                if let Err(mpsc::error::TrySendError::Full(_)) = sent {
                    queue.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Msg::Call(call) => on_call(inner, handle, call),
        Msg::Cancel(cancel) => {
            if let Some(served) = lock(&inner.state).served.remove(&cancel.id) {
                if let Some(abort) = served.abort {
                    abort.abort();
                }
                if let Some(gate) = served.gate {
                    gate.close();
                }
            }
        }
        Msg::Credit(credit) => {
            let gate = lock(&inner.state)
                .served
                .get(&credit.id)
                .and_then(|served| served.gate.clone());
            if let Some(gate) = gate {
                gate.add(u64::from(credit.bytes));
            }
        }
        Msg::CredentialUpdate(update) => {
            *lock(&inner.credential) = Some(Credential::from_bytes(update.credential));
        }
        Msg::ConnectionError(error) => {
            // bad_frame leaves the connection open; anything else is followed by
            // the close, and pending calls fail with this error.
            if error.code != codes::BAD_FRAME {
                inner.shutdown(CloseInfo {
                    error: Some(BusError::new(error.code, error.message)),
                    ..CloseInfo::default()
                });
            }
        }
        Msg::Hello(_)
        | Msg::HelloAck(_)
        | Msg::Subscribe(_)
        | Msg::Unsubscribe(_)
        | Msg::Activate(_) => {}
    }
}

fn on_reply(inner: &Inner, reply: wire::Reply) {
    let mut state = lock(&inner.state);
    if let Some(tx) = state.calls.remove(&reply.id) {
        drop(state);
        let result = match reply.result {
            Some(wire::reply::Result::Error(error)) => Err(error.into()),
            Some(wire::reply::Result::Payload(payload)) => Ok(Reply {
                payload,
                content_type: reply.content_type,
            }),
            None => Ok(Reply {
                payload: Bytes::new(),
                content_type: reply.content_type,
            }),
        };
        let _ = tx.send(result);
    } else if let Some(stream) = state.streams.remove(&reply.id) {
        drop(state);
        let error = match reply.result {
            Some(wire::reply::Result::Error(error)) => error.into(),
            _ => BusError::bad_reply("a stream was answered with a reply instead of chunks"),
        };
        let _ = stream.tx.send(Err(error));
    }
}

fn on_chunk(inner: &Inner, chunk: wire::Chunk) {
    let mut state = lock(&inner.state);
    let Some(stream) = state.streams.get_mut(&chunk.id) else {
        return; // A chunk of a stream given up.
    };
    stream.received += chunk.payload.len() as u64;
    if stream.received > stream.allowed {
        let stream = state.streams.remove(&chunk.id).expect("present");
        drop(state);
        let _ = stream.tx.send(Err(BusError::bad_reply(
            "the provider sent more than the stream's credit",
        )));
        inner.send_now(Msg::Cancel(wire::Cancel {
            id: chunk.id,
            reason: "credit overrun".into(),
        }));
        return;
    }
    let _ = stream.tx.send(Ok(StreamItem::Chunk(Chunk {
        payload: chunk.payload,
        content_encoding: chunk.content_encoding,
    })));
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        return (*text).to_owned();
    }
    if let Some(text) = panic.downcast_ref::<String>() {
        return text.clone();
    }
    "the handler panicked".into()
}

fn on_call(inner: &Arc<Inner>, handle: &Weak<Handle>, call: wire::Call) {
    let Some(handle) = handle.upgrade() else {
        // The connection is going away: answer rather than leave the caller
        // waiting for its deadline.
        inner.send_now(error_reply(
            call.id,
            BusError::unavailable("the connection is closing"),
        ));
        return;
    };
    let bus = Bus(handle);
    let Some(handler) = inner.handlers.get(&call.method).cloned() else {
        inner.send_now(Msg::Reply(wire::Reply {
            id: call.id,
            result: Some(wire::reply::Result::Error(wire::Error {
                code: codes::NOT_SUPPORTED.into(),
                message: format!("{} does not provide {}", inner.ack.identity, call.method),
            })),
            content_type: String::new(),
        }));
        return;
    };
    let id = call.id;
    let window = match call.window {
        0 => match inner.ack.stream_window {
            0 => limits::STREAM_WINDOW,
            window => window,
        },
        window => window,
    };
    let request = Request {
        id,
        method: call.method,
        from: call.from,
        payload: call.payload,
        content_type: call.content_type,
        trace_id: call.trace_id,
        timeout_ms: call.timeout_ms,
        deadline: (call.timeout_ms > 0)
            .then(|| Instant::now() + Duration::from_millis(u64::from(call.timeout_ms))),
        credential: call.credential,
        window,
        bus: bus.clone(),
    };
    let gate =
        matches!(handler, Handler::Stream(_)).then(|| Arc::new(CreditGate::new(u64::from(window))));
    {
        let mut state = lock(&inner.state);
        if state.closed {
            return;
        }
        state.served.insert(
            id,
            Served {
                abort: None,
                gate: gate.clone(),
            },
        );
    }
    let task_bus = bus.clone();
    let task = tokio::spawn(async move {
        let outcome = match handler {
            Handler::Unary(handler) => {
                // The whole call, so a handler that panics before returning its
                // future is caught too.
                match AssertUnwindSafe(async move { handler(request).await })
                    .catch_unwind()
                    .await
                {
                    Ok(Ok(reply)) => Msg::Reply(wire::Reply {
                        id,
                        result: Some(wire::reply::Result::Payload(reply.payload)),
                        content_type: reply.content_type,
                    }),
                    Ok(Err(error)) => error_reply(id, error),
                    Err(panic) => error_reply(id, BusError::handler_error(panic_message(panic))),
                }
            }
            Handler::Stream(handler) => {
                let writer = StreamWriter {
                    bus: task_bus.clone(),
                    id,
                    gate: gate.expect("a stream has a gate"),
                    window,
                };
                match AssertUnwindSafe(async move { handler(request, writer).await })
                    .catch_unwind()
                    .await
                {
                    Ok(Ok(end)) => Msg::End(wire::End {
                        id,
                        payload: end.unwrap_or_default(),
                    }),
                    Ok(Err(error)) => error_reply(id, error),
                    Err(panic) => error_reply(id, BusError::handler_error(panic_message(panic))),
                }
            }
        };
        let inner = task_bus.inner();
        // A call cancelled meanwhile gets no answer.
        let answer = lock(&inner.state).served.remove(&id).is_some();
        // An answer over the limit: say so instead.
        if answer
            && let Err(error) = inner.send(outcome).await
            && error.code == codes::TOO_LARGE
        {
            let _ = inner.send(error_reply(id, error)).await;
        }
    });
    if let Some(served) = lock(&inner.state).served.get_mut(&id) {
        served.abort = Some(task.abort_handle());
    }
}

fn error_reply(id: u64, error: BusError) -> Msg {
    Msg::Reply(wire::Reply {
        id,
        result: Some(wire::reply::Result::Error(error.into())),
        content_type: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Transport, BusError> {
        Transport::from_lookup(lookup, &AtomicBool::new(false))
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn the_environment_names_the_transport() {
        let Transport::Socket { path, token } = lookup(vars(&[
            ("TILR_BUS_SOCK", "/run/t.sock"),
            ("TILR_BUS_TOKEN", "ab"),
        ]))
        .unwrap() else {
            panic!("not a socket");
        };
        assert_eq!(path, PathBuf::from("/run/t.sock"));
        assert_eq!(token, "ab");

        let credential = Credential::from_bytes(&b"\x00\xfftoken"[..]);
        let Transport::WebSocket {
            url,
            credential: got,
            ..
        } = lookup(vars(&[
            ("TILR_BUS_URL", "ws://127.0.0.1:7890/"),
            ("TILR_BUS_CREDENTIAL", &credential.to_base64url()),
        ]))
        .unwrap()
        else {
            panic!("not a websocket");
        };
        assert_eq!(url, "ws://127.0.0.1:7890/");
        assert_eq!(got, Some(credential));

        let Transport::UnixWebSocket {
            path, credential, ..
        } = lookup(vars(&[("TILR_BUS_URL", "unix:/tmp/tilrd.sock")])).unwrap()
        else {
            panic!("not a unix websocket");
        };
        assert_eq!(path, PathBuf::from("/tmp/tilrd.sock"));
        assert_eq!(credential, None);
    }

    #[test]
    fn without_a_bus_in_the_environment_connecting_is_unavailable() {
        let error = lookup(vars(&[])).unwrap_err();
        assert_eq!(error.code, codes::UNAVAILABLE);
        for name in [
            "TILR_BUS_FD",
            "TILR_BUS_SOCK",
            "TILR_BUS_TOKEN",
            "TILR_BUS_URL",
        ] {
            assert!(error.message.contains(name), "{error}");
        }
        let error = lookup(vars(&[("TILR_BUS_SOCK", "/x")])).unwrap_err();
        assert!(error.message.contains("TILR_BUS_TOKEN"), "{error}");
        let error = lookup(vars(&[("TILR_BUS_FD", "x")])).unwrap_err();
        assert_eq!(error.code, codes::BAD_REQUEST);
    }

    #[test]
    fn the_bus_descriptor_must_be_an_open_socket() {
        let taken = AtomicBool::new(false);
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let fd = file.as_raw_fd().to_string();
        let error = Transport::from_lookup(vars(&[("TILR_BUS_FD", &fd)]), &taken).unwrap_err();
        assert_eq!(error.code, codes::BAD_REQUEST);
        assert!(error.message.contains("is not a socket"), "{error}");
        let error =
            Transport::from_lookup(vars(&[("TILR_BUS_FD", "1000000")]), &taken).unwrap_err();
        assert!(error.message.contains("not an open descriptor"), "{error}");
        // Neither refusal took anything: the file is still open.
        assert!(!taken.load(Ordering::SeqCst));
        assert!(file.metadata().is_ok());
    }

    #[test]
    fn the_bus_descriptor_is_taken_once() {
        let taken = AtomicBool::new(false);
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd = ours.as_raw_fd().to_string();
        // The transport owns it now.
        std::mem::forget(ours);
        let first = Transport::from_lookup(vars(&[("TILR_BUS_FD", &fd)]), &taken).unwrap();
        assert!(matches!(first, Transport::Fd(_)));
        let error = Transport::from_lookup(vars(&[("TILR_BUS_FD", &fd)]), &taken).unwrap_err();
        assert_eq!(error.code, codes::BAD_REQUEST);
        assert!(error.message.contains("was taken already"), "{error}");
        drop(first);
        drop(theirs);
    }

    #[test]
    fn a_transport_never_shows_its_tokens() {
        let shown = [
            Transport::Socket {
                path: "/run/t.sock".into(),
                token: "instance-secret".into(),
            },
            Transport::WebSocket {
                url: "ws://127.0.0.1:1/".into(),
                access_token: Some("access-secret".into()),
                credential: Some(Credential::from_bytes(&b"credential-secret"[..])),
                origin: None,
            },
            Transport::UnixWebSocket {
                path: "/run/tilrd.sock".into(),
                credential: Some(Credential::from_bytes(&b"credential-secret"[..])),
                origin: None,
            },
        ]
        .map(|transport| format!("{transport:?}"));
        for text in &shown {
            assert!(!text.contains("secret"), "{text}");
        }
        assert!(shown[0].contains("/run/t.sock") && shown[0].contains("<redacted>"));
        assert!(shown[1].contains("ws://127.0.0.1:1/"), "{}", shown[1]);
    }

    #[test]
    fn a_credential_is_base64url_and_never_shown() {
        let credential = Credential::from_bytes(&b"\xfb\xff secret"[..]);
        let text = credential.to_base64url();
        assert!(!text.contains('+') && !text.contains('/') && !text.contains('='));
        assert_eq!(
            Credential::from_base64url(&format!(" {text}== ")).unwrap(),
            credential
        );
        assert_eq!(format!("{credential:?}"), "Credential(9 bytes)");
        assert!(Credential::from_base64url("not base64!").is_err());
    }

    #[test]
    fn a_zstd_chunk_is_decompressed_on_reading() {
        let plain = "frame ".repeat(100);
        let chunk = Chunk {
            payload: Bytes::from(zstd::encode_all(plain.as_bytes(), 1).unwrap()),
            content_encoding: codec::ENCODING_ZSTD.into(),
        };
        assert_eq!(chunk.bytes().unwrap(), plain.as_bytes());
        let unknown = Chunk {
            payload: Bytes::new(),
            content_encoding: "br".into(),
        };
        assert_eq!(unknown.bytes().unwrap_err().code, codes::BAD_REPLY);
    }
}
