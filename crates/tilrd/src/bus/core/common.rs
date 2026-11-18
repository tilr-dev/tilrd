//! What the namespaces of `bus-core-services` (`server`, `fs`, `exec`,
//! `clipboard`) share: the JSON of their messages ([`json_message`]), a unary
//! method run with the caller's order kept ([`unary`]), the supervisor's part
//! of a call ([`queue`], [`asked`]), the cleanup when a caller's connection
//! ends ([`on_connection_end`]), their errors, and a stream's data cut to its
//! window ([`room`], [`split`]).
//!
//! Owner: `bus-core-services`.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use bytes::Bytes;
use prost::Message;
use tilr_bus::{BusError, Reply};
use tokio::sync::oneshot;

use super::SupervisorHandle;
use super::json::{self, Json};
use crate::bus::broker::ConnState;
use crate::bus::identity::CallCtx;
use crate::bus::streams::NativeMethod;
use crate::supervisor::Supervisor;

/// The proto3 JSON mapping ([`Json`]) of a message, from its fields: each
/// `field: kind "jsonName"`, where kind is `string`, `strings`, `bool`, `u32`,
/// `i32`, `u64`, `bytes`, `message(T)`, `messages(T)` or `enumeration(E)`. The
/// proto name is the field's own. Every field of the message is listed.
macro_rules! json_message {
    ($ty:path { $($field:ident : $kind:ident $(($arg:path))? $json:literal),* $(,)? }) => {
        impl $crate::bus::core::json::Json for $ty {
            fn to_json(&self) -> ::serde_json::Value {
                #[allow(unused_mut, reason = "a message without fields")]
                let mut object = ::serde_json::Map::new();
                $(
                    if let Some(value) =
                        $crate::bus::core::common::json_message!(@write $kind $(($arg))?, &self.$field)
                    {
                        object.insert($json.into(), value);
                    }
                )*
                ::serde_json::Value::Object(object)
            }

            fn from_json(value: &::serde_json::Value) -> Result<Self, String> {
                #[allow(unused_variables, reason = "a message without fields")]
                let f = $crate::bus::core::json::Fields::of(value)?;
                Ok(Self {
                    $(
                        $field: $crate::bus::core::common::json_message!(
                            @read $kind $(($arg))?, f, $json, stringify!($field)
                        )?,
                    )*
                })
            }
        }
    };
    (@write string, $v:expr) => { Some(::serde_json::json!($v)) };
    (@write strings, $v:expr) => { Some(::serde_json::json!($v)) };
    (@write bool, $v:expr) => { Some(::serde_json::json!($v)) };
    (@write u32, $v:expr) => { Some(::serde_json::json!($v)) };
    (@write i32, $v:expr) => { Some(::serde_json::json!($v)) };
    (@write u64, $v:expr) => { Some($crate::bus::core::json::int64($v)) };
    (@write bytes, $v:expr) => { Some($crate::bus::core::json::bytes($v)) };
    (@write message($t:path), $v:expr) => {
        ($v).as_ref().map($crate::bus::core::json::Json::to_json)
    };
    (@write messages($t:path), $v:expr) => { Some($crate::bus::core::json::messages($v)) };
    (@write enumeration($e:path), $v:expr) => {
        Some($crate::bus::core::json::enumeration(
            *$v,
            <$e>::try_from(*$v).ok().map(|e| e.as_str_name()),
        ))
    };
    (@read string, $f:expr, $json:literal, $proto:expr) => { $f.string($json, $proto) };
    (@read strings, $f:expr, $json:literal, $proto:expr) => { $f.strings($json, $proto) };
    (@read bool, $f:expr, $json:literal, $proto:expr) => { $f.bool($json, $proto) };
    (@read u32, $f:expr, $json:literal, $proto:expr) => { $f.u32($json, $proto) };
    (@read i32, $f:expr, $json:literal, $proto:expr) => { $f.i32($json, $proto) };
    (@read u64, $f:expr, $json:literal, $proto:expr) => { $f.u64($json, $proto) };
    (@read bytes, $f:expr, $json:literal, $proto:expr) => { $f.bytes($json, $proto) };
    (@read message($t:path), $f:expr, $json:literal, $proto:expr) => {
        $f.message::<$t>($json, $proto)
    };
    (@read messages($t:path), $f:expr, $json:literal, $proto:expr) => {
        $f.messages::<$t>($json, $proto)
    };
    (@read enumeration($e:path), $f:expr, $json:literal, $proto:expr) => {
        $f.enumeration($json, $proto, |name| {
            <$e>::from_str_name(name).map(|e| e as i32)
        })
    };
}
pub(crate) use json_message;

/// Queue `f` on the supervisor's task now and get what it returns later (as
/// `panes.rs` does: the future of `SupervisorHandle::ask` keeps the handle
/// borrowed, so it cannot leave a handler's synchronous part).
pub(super) fn queue<R: Send + 'static>(
    supervisor: &SupervisorHandle,
    f: impl FnOnce(&mut Supervisor) -> R + Send + 'static,
) -> Result<oneshot::Receiver<R>, BusError> {
    let (tx, rx) = oneshot::channel();
    supervisor.with(move |s| {
        let _ = tx.send(f(s));
    })?;
    Ok(rx)
}

/// What a queued part answered; `unavailable` once the supervisor is gone.
pub(super) async fn asked<R>(rx: oneshot::Receiver<R>) -> Result<R, BusError> {
    rx.await
        .map_err(|_| BusError::unavailable("tilrd is shutting down"))
}

/// A unary method: the request decoded (JSON or protobuf, B9), `handler`
/// called in the order the caller's calls arrive (its synchronous part runs
/// before the next call's: work whose order matters is queued there), the
/// answer in the request's encoding.
pub(super) fn unary<Req, Resp, F, Fut>(method: &'static str, handler: F) -> NativeMethod
where
    Req: Message + Default + Json + Send + 'static,
    Resp: Message + Json + Send + 'static,
    F: Fn(&CallCtx, Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Resp, BusError>> + Send + 'static,
{
    NativeMethod::unary(method, move |ctx: CallCtx, payload: Bytes| {
        let answer = json::decode::<Req>(&ctx, &payload).map(|request| handler(&ctx, request));
        async move {
            let response = answer?.await?;
            Ok::<Reply, BusError>(json::reply(&ctx, &response))
        }
    })
}

/// The connections whose end each namespace waits for (a slot).
#[derive(Default)]
struct Ends(HashMap<&'static str, HashSet<u64>>);

/// On the supervisor's task: run `f` there once the connection `conn` ends,
/// once per namespace `what` and connection, however often this is called.
/// For what a caller leaves behind: watches, uploads, programs. `false` when
/// the connection is gone already (`f` is not run then): the caller keeps
/// nothing for it and refuses the call, since nothing would clean it up.
#[must_use]
pub(super) fn on_connection_end(
    s: &mut Supervisor,
    what: &'static str,
    conn: u64,
    f: fn(&mut Supervisor, u64),
) -> bool {
    let Some(broker) = s.bus.broker().cloned() else {
        return false;
    };
    let supervisor = handle(s);
    if s.bus
        .slot::<Ends>()
        .0
        .get(what)
        .is_some_and(|conns| conns.contains(&conn))
    {
        // Watched already; an end seen meanwhile is queued behind this and
        // cleans up what the caller keeps now.
        return true;
    }
    let Some(mut state) = broker.watch_connection(conn) else {
        return false;
    };
    s.bus.slot::<Ends>().0.entry(what).or_default().insert(conn);
    tokio::spawn(async move {
        let _ = state
            .wait_for(|state| matches!(state, ConnState::Ended(_)))
            .await;
        let _ = supervisor.with(move |s: &mut Supervisor| {
            if let Some(conns) = s.bus.slot::<Ends>().0.get_mut(what) {
                conns.remove(&conn);
            }
            f(s, conn);
        });
    });
    true
}

/// The supervisor's handle, for what runs on its task and queues more there.
pub(super) fn handle(s: &Supervisor) -> SupervisorHandle {
    SupervisorHandle::new(s.events.clone())
}

/// A connection's share of something limited is used up: `busy`.
pub(super) fn limit(message: impl Into<String>) -> BusError {
    BusError::new(tilr_bus::codes::BUSY, message)
}

/// The caller's connection.
pub(super) fn conn_of(ctx: &CallCtx) -> u64 {
    ctx.principal.connection_id
}

/// A failure meant to be shown to the user, as `bad_request`.
pub(super) fn refused(message: impl Into<String>) -> BusError {
    BusError::bad_request(message)
}

/// The payload room a chunk leaves in the stream's window once `empty` (the
/// chunk's message without its data) is encoded: what the data may take.
pub(super) fn room(window: u64, empty: usize) -> usize {
    // The data's tag and length.
    let overhead = empty + 6;
    usize::try_from(window)
        .unwrap_or(usize::MAX)
        .saturating_sub(overhead)
        .max(1)
}

/// `data` in pieces of at most `room` bytes; one empty piece for no data.
pub(super) fn split(data: Bytes, room: usize) -> Vec<Bytes> {
    if data.len() <= room {
        return vec![data];
    }
    (0..data.len())
        .step_by(room)
        .map(|at| data.slice(at..data.len().min(at + room)))
        .collect()
}
