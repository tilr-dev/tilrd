//! The native participant's methods and the provider side of its streams (design
//! B7; change `bus-broker-core`, "Traits").
//!
//! Owner: `bus-broker-core`, then `bus-streams-events` (credits, the connection
//! window, deadlines, "no credit" as lagging).
//!
//! A stream's producer sends at most the credit its reader gave: `Call.window` (0:
//! 1 MiB, at most 16 MiB) plus what the reader returned with `Credit` (B7).
//! Credit counts `Chunk.payload` bytes as the producer hands them over: the
//! payload of a [`Chunk::zstd`] counts as compressed, as `tilr-bus` counts it on
//! both ends; the compression of WebSocket messages never counts.
//! [`StreamOut::send`] waits for it, and for a bulk stream also for the reader's
//! connection window (1 MiB sent and not yet credited over all its bulk streams)
//! and for room in its bulk lane (1024 envelopes queued); [`StreamOut::try_send`]
//! does not wait. A chunk larger than the stream's [`window`](StreamOut::window)
//! (and than the credit it holds) can never be sent: the stream ends with
//! `too_large`, and `send` and `try_send` answer `Closed` (never `Full`), so a
//! latest-wins producer does not loop on it; split to the window. An empty
//! chunk is sent like any other. Checked, taken and queued under one
//! lock: a chunk never comes after its stream's end. Chunks of an interactive
//! stream go on the reader connection's interactive lane (1024 envelopes or
//! 32 MiB, full: the connection is closed 4001). The end ([`StreamOut::end`],
//! [`StreamOut::fail`]) never waits for credit.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use tilr_bus::bus::{Kind, Lane, MethodSpec, Select};
use tilr_bus::{BusError, Reply};

use super::broker::BrokerHandle;
use super::identity::{CallCtx, CancelToken};

/// A boxed, sendable future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A unary handler: the request's payload in, the reply out.
pub type UnaryFn = Arc<dyn Fn(CallCtx, Bytes) -> BoxFuture<Result<Reply, BusError>> + Send + Sync>;
/// A stream handler: writes chunks to its [`StreamOut`], ends it with
/// [`StreamOut::end`] or [`StreamOut::fail`].
pub type StreamFn = Arc<dyn Fn(CallCtx, Bytes, StreamOut) -> BoxFuture<()> + Send + Sync>;

/// How a native method answers.
///
/// The broker calls the handler function in the order the calls arrive from one
/// connection, on that connection's reader, and runs the future it returns on a
/// task of its own. Work whose order matters (`pane.input`: the per-pair order of
/// B5) is started in the function, before the future is returned (a
/// [`super::SupervisorHandle`] command sent there keeps its place).
#[derive(Clone)]
pub enum NativeHandler {
    Unary(UnaryFn),
    Stream(StreamFn),
}

/// A method of the native participant `tilrd`, added with
/// [`super::Registry::add`]. A feature turned off adds nothing (B14).
#[derive(Clone)]
pub struct NativeMethod {
    pub spec: MethodSpec,
    pub handler: NativeHandler,
}

impl NativeMethod {
    /// A unary method; `handler` answers with a reply or an error.
    pub fn unary<F, Fut>(method: &str, handler: F) -> Self
    where
        F: Fn(CallCtx, Bytes) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Reply, BusError>> + Send + 'static,
    {
        Self {
            spec: MethodSpec {
                method: method.into(),
                kind: Kind::Unary as i32,
                lane: Lane::Interactive as i32,
                select: Select::RoundRobin as i32,
            },
            handler: NativeHandler::Unary(Arc::new(move |ctx, payload| {
                Box::pin(handler(ctx, payload))
            })),
        }
    }

    /// A stream on `lane`.
    pub fn stream<F, Fut>(method: &str, lane: Lane, handler: F) -> Self
    where
        F: Fn(CallCtx, Bytes, StreamOut) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Self {
            spec: MethodSpec {
                method: method.into(),
                kind: Kind::Stream as i32,
                lane: lane as i32,
                select: Select::RoundRobin as i32,
            },
            handler: NativeHandler::Stream(Arc::new(move |ctx, payload, out| {
                Box::pin(handler(ctx, payload, out))
            })),
        }
    }

    /// A unary method whose function is written as [`UnaryFn`] directly: for a
    /// handler that does ordered work before it returns its future.
    pub fn unary_fn(method: &str, handler: UnaryFn) -> Self {
        Self {
            spec: MethodSpec {
                method: method.into(),
                kind: Kind::Unary as i32,
                lane: Lane::Interactive as i32,
                select: Select::RoundRobin as i32,
            },
            handler: NativeHandler::Unary(handler),
        }
    }
}

/// One chunk of a stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chunk {
    pub payload: Bytes,
    /// `zstd` when the producer compressed the payload (B9); else empty.
    pub content_encoding: String,
}

impl Chunk {
    pub fn new(payload: impl Into<Bytes>) -> Self {
        Self {
            payload: payload.into(),
            content_encoding: String::new(),
        }
    }

    /// A payload its producer compressed with zstd: sent plain on a WebSocket.
    pub fn zstd(payload: impl Into<Bytes>) -> Self {
        Self {
            payload: payload.into(),
            content_encoding: tilr_bus::codec::ENCODING_ZSTD.into(),
        }
    }
}

/// The stream is over: the caller cancelled it or its connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed;

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the stream is closed")
    }
}

/// Why [`StreamOut::try_send`] did not send.
#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError {
    /// Sending would wait (no credit, no connection window, or no room in the
    /// bulk lane); the chunk is handed back.
    Full(Chunk),
    Closed,
}

/// The provider's side of one stream of the native participant.
///
/// Dropped without [`end`](Self::end) or [`fail`](Self::fail) (the handler
/// returned early or panicked), the stream ends with `handler_error`.
pub struct StreamOut {
    pub(super) broker: BrokerHandle,
    /// The caller's connection and its id of the call.
    pub(super) conn: u64,
    pub(super) id: u64,
    /// This call of `id`, so a late drop does not end a newer call of the same id.
    pub(super) generation: u64,
    pub(super) lane: Lane,
    /// The window the reader opened the stream with.
    pub(super) window: u64,
    pub(super) cancel: CancelToken,
    pub(super) done: bool,
}

impl StreamOut {
    /// The caller's id of the stream.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Send a chunk; waits for the reader's credit (and for a bulk stream, for
    /// its connection window and room in its bulk lane). `Closed` once the stream
    /// is over (cancelled, timed out, ended with `too_large`, its reader gone).
    pub async fn send(&self, chunk: Chunk) -> Result<(), Closed> {
        self.broker
            .stream_chunk(self.conn, self.id, self.generation, self.lane, chunk)
            .await
    }

    /// The window the reader opened the stream with: no chunk may be larger
    /// (unless the credit it holds is), a latest-wins producer splits to it.
    pub fn window(&self) -> u64 {
        self.window
    }

    /// Send a chunk if that does not wait: `Full` (the chunk handed back) when
    /// it would, `Closed` once the stream is over or the chunk can never go.
    pub fn try_send(&self, chunk: Chunk) -> Result<(), TrySendError> {
        self.broker
            .try_stream_chunk(self.conn, self.id, self.generation, self.lane, chunk)
    }

    /// Whether the consumer lags: its connection's interactive lane has held a
    /// packet for over 250 ms, or the stream has no credit left (a bulk stream:
    /// or the connection window is spent).
    pub fn is_lagging(&self) -> bool {
        self.broker
            .stream_lagging(self.conn, self.id, self.generation)
    }

    /// Returns once the consumer's interactive lane is written out and the stream
    /// has some credit again, at least a byte (and for a bulk stream the
    /// connection window and the bulk lane have room) (T4: then a full frame is
    /// due); at once when the stream or the connection is gone. A producer
    /// whose next chunk is larger waits with [`caught_up_for`](Self::caught_up_for).
    pub async fn caught_up(&self) {
        self.caught_up_for(1).await;
    }

    /// [`caught_up`](Self::caught_up) for a chunk of `bytes`: returns once one
    /// could be sent now; at once when `bytes` is larger than the window (a
    /// send of it ends the stream `too_large`).
    pub async fn caught_up_for(&self, bytes: u64) {
        self.broker
            .stream_caught_up(self.conn, self.id, self.generation, bytes)
            .await;
    }

    /// Cancelled when the caller cancels the stream or its connection ends.
    pub fn cancelled(&self) -> &CancelToken {
        &self.cancel
    }

    /// End the stream with `End` (an optional final payload).
    pub fn end(mut self, payload: Bytes) {
        self.done = true;
        self.broker
            .stream_end(self.conn, self.id, self.generation, self.lane, Ok(payload));
    }

    /// End the stream with an error `Reply`.
    pub fn fail(mut self, error: BusError) {
        self.done = true;
        self.broker
            .stream_end(self.conn, self.id, self.generation, self.lane, Err(error));
    }
}

impl Drop for StreamOut {
    fn drop(&mut self) {
        if !self.done {
            self.broker.stream_end(
                self.conn,
                self.id,
                self.generation,
                self.lane,
                Err(BusError::handler_error(
                    "the stream's handler ended without End",
                )),
            );
        }
    }
}

impl std::fmt::Debug for StreamOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamOut")
            .field("conn", &self.conn)
            .field("id", &self.id)
            .field("lane", &self.lane)
            .finish()
    }
}
