//! The native participant's methods and the provider side of its streams (design
//! B7; change `bus-broker-core`, "Traits").
//!
//! Owner: `bus-broker-core`, then `bus-streams-events`, which adds credits (a
//! [`StreamOut::send`] that waits for them), lanes bounded in bytes, deadlines and
//! "no credit" as lagging behind these types.
//!
//! In this change a stream has unlimited credit: chunks of an interactive stream go
//! on the consumer connection's interactive lane (1024 envelopes, full: the
//! connection is closed 4001), chunks of a bulk stream wait for room in its bulk
//! lane (4 MiB).

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
    /// Sending would wait (no room, or no credit once `bus-streams-events`
    /// enforces it); the chunk is handed back.
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
    pub(super) cancel: CancelToken,
    pub(super) done: bool,
}

impl StreamOut {
    /// The caller's id of the stream.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Send a chunk; waits for room in a bulk lane (and for credit once
    /// `bus-streams-events` enforces it).
    pub async fn send(&self, chunk: Chunk) -> Result<(), Closed> {
        self.broker
            .stream_chunk(self.conn, self.id, self.generation, self.lane, chunk)
            .await
    }

    /// Send a chunk if that does not wait.
    pub fn try_send(&self, chunk: Chunk) -> Result<(), TrySendError> {
        self.broker
            .try_stream_chunk(self.conn, self.id, self.generation, self.lane, chunk)
    }

    /// Whether the consumer lags: its connection's interactive lane has held a
    /// packet for over 250 ms (`bus-streams-events` adds "no credit").
    pub fn is_lagging(&self) -> bool {
        self.broker.conn_lagging(self.conn)
    }

    /// Returns once the consumer's interactive lane is written out (T4: then a
    /// full frame is due); at once when it is, or when the connection is gone.
    pub async fn caught_up(&self) {
        self.broker.conn_caught_up(self.conn).await;
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
