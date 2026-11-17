//! `server.*` (T2): `server.info`, `server.reload`, `server.update`,
//! `server.command`; and what the namespaces of `bus-core-services` share: the
//! JSON of their messages ([`json_message`]), a unary method run with the
//! caller's order kept ([`unary`]), the supervisor's part of a call ([`queue`])
//! and the cleanup when a caller's connection ends ([`on_connection_end`]).
//!
//! Owner: `bus-core-services`.
//!
//! `server.update` and `server.command` run the server's own executable as an
//! exec (`exec::Execs`, as `tilr.v1`'s `ExecStart.server_command` did) and
//! stream it as `exec.run` does (`super::exec::stream_exec`). `server.update`
//! is `tilrd update`, which reloads the server once it installed a release.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use bytes::Bytes;
use prost::Message;
use tilr_bus::bus::Lane;
use tilr_bus::server::{
    CommandRequest, InfoReply, InfoRequest, ReloadReply, ReloadRequest, UpdateRequest,
};
use tilr_bus::{BusError, Reply, methods};
use tokio::sync::oneshot;

use super::json::{self, Json};
use super::{Core, Registry, SupervisorHandle};
use crate::bus::authz::AuthzRequest;
use crate::bus::broker::ConnState;
use crate::bus::identity::{CallCtx, Identity, Transport};
use crate::bus::streams::NativeMethod;
use crate::exec::Program;
use crate::supervisor::{
    Event, FEATURE_AGENT, FEATURE_CLIPBOARD, FEATURE_EXEC, FEATURE_FILES, FEATURE_HOST_EVENTS,
    Supervisor,
};

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
                        $crate::bus::core::server::json_message!(@write $kind $(($arg))?, &self.$field)
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
                        $field: $crate::bus::core::server::json_message!(
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

json_message!(InfoRequest {});
json_message!(InfoReply {
    server_version: string "serverVersion",
    features: strings "features",
    clipboard_max_bytes: u64 "clipboardMaxBytes",
    server_id: string "serverId",
    fs_max_bytes: u64 "fsMaxBytes",
    update_channel: string "updateChannel",
});
json_message!(ReloadRequest {});
json_message!(ReloadReply {});
json_message!(UpdateRequest {});
json_message!(CommandRequest {
    argv: strings "argv",
    cwd: string "cwd",
    env: strings "env",
    stdin: bool "stdin",
    timeout_ms: u32 "timeoutMs",
});

// ---------------------------------------------------------------------------
// Shared by the namespaces of bus-core-services
// ---------------------------------------------------------------------------

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
    let Some(supervisor) = s.bus.slot::<HandleSlot>().0.clone() else {
        return false;
    };
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

/// A connection's share of something limited is used up: `busy`.
pub(super) fn limit(message: impl Into<String>) -> BusError {
    BusError::new(tilr_bus::codes::BUSY, message)
}

/// The supervisor's handle, for what runs on its task and needs to queue more.
#[derive(Default)]
struct HandleSlot(Option<SupervisorHandle>);

/// The caller's connection.
pub(super) fn conn_of(ctx: &CallCtx) -> u64 {
    ctx.principal.connection_id
}

/// A failure meant to be shown to the user, as `bad_request`.
pub(super) fn refused(message: impl Into<String>) -> BusError {
    BusError::bad_request(message)
}

// ---------------------------------------------------------------------------
// server.*
// ---------------------------------------------------------------------------

/// Add `server.*`; `server.update` and `server.command` run even with the
/// `exec` feature off, as `tilr.v1`'s server commands do.
pub fn register(registry: &mut Registry, core: &Core) {
    let handle = core.supervisor.clone();
    let _ = core.supervisor.with(move |s| {
        s.bus.slot::<HandleSlot>().0 = Some(handle);
    });
    let supervisor = core.supervisor.clone();
    registry.add(unary(
        methods::SERVER_INFO.name,
        move |_: &CallCtx, _: InfoRequest| {
            let asked_info = queue(&supervisor, info);
            async move { asked(asked_info?).await? }
        },
    ));
    let supervisor = core.supervisor.clone();
    let broker = core.broker.clone();
    registry.add(unary(
        methods::SERVER_RELOAD.name,
        move |ctx: &CallCtx, _: ReloadRequest| {
            let allowed = may_reload(&broker, ctx).and_then(|()| {
                tracing::info!(identity = %ctx.from, "reload asked on the bus");
                // As SIGUSR1: the reload first checks the new executable,
                // which takes longer than this answer to go out.
                queue(&supervisor, |s| {
                    let _ = s.events.send(Event::Reload);
                })
            });
            async move {
                asked(allowed?).await?;
                Ok(ReloadReply {})
            }
        },
    ));
    let update = core.clone();
    registry.add(NativeMethod::stream(
        methods::SERVER_UPDATE.name,
        Lane::Bulk,
        move |ctx, payload, out| {
            let core = update.clone();
            async move {
                if let Err(err) = json::decode::<UpdateRequest>(&ctx, &payload)
                    .and_then(|_| may_reload(&core.broker, &ctx))
                {
                    return out.fail(err);
                }
                let program = Program {
                    argv: vec![exe(), "update".into()],
                    ..Program::default()
                };
                super::exec::stream_exec(core, ctx, program, out).await;
            }
        },
    ));
    let command = core.clone();
    registry.add(NativeMethod::stream(
        methods::SERVER_COMMAND.name,
        Lane::Bulk,
        move |ctx, payload, out| {
            let core = command.clone();
            async move {
                let request = match json::decode::<CommandRequest>(&ctx, &payload) {
                    Ok(request) => request,
                    Err(err) => return out.fail(err),
                };
                // As `tilr.v1`: the owner on the Unix socket only.
                let local =
                    ctx.principal.transport == Transport::UnixSocket && ctx.from == Identity::Owner;
                if !local {
                    return out.fail(BusError::new(
                        tilr_bus::codes::DENIED,
                        "server commands are taken on the Unix socket only",
                    ));
                }
                let mut argv = vec![exe()];
                argv.extend(request.argv);
                let program = Program {
                    argv,
                    cwd: request.cwd,
                    env: request.env,
                    stdin: request.stdin,
                    timeout_ms: request.timeout_ms,
                };
                super::exec::stream_exec(core, ctx, program, out).await;
            }
        },
    ));
}

fn exe() -> String {
    crate::exe().to_string_lossy().into_owned()
}

/// `server.reload` and `server.update`: the owner, or the `control` right
/// `reload`.
fn may_reload(broker: &crate::bus::BrokerHandle, ctx: &CallCtx) -> Result<(), BusError> {
    tracing::info!(target: "tilrd::bus::audit", identity = %ctx.from, method = ctx.method, "control");
    broker
        .authorizer()
        .authorize(
            &ctx.principal,
            ctx.call_credential.as_deref(),
            &AuthzRequest::Control {
                action: "reload",
                instance: None,
            },
        )
        .map_err(BusError::from)
}

/// `server.info`: as `tilr.v1`'s `HelloAck`.
fn info(s: &mut Supervisor) -> Result<InfoReply, BusError> {
    let clipboard_max_bytes = s.clipboard.as_ref().map(|c| c.max_bytes());
    let fs_max_bytes = s.files.as_ref().map(|f| f.max_bytes());
    let features = [
        (FEATURE_CLIPBOARD, clipboard_max_bytes.is_some()),
        (FEATURE_FILES, fs_max_bytes.is_some()),
        (FEATURE_HOST_EVENTS, s.host_events),
        (FEATURE_AGENT, s.agent.is_some()),
        (FEATURE_EXEC, s.exec),
    ];
    Ok(InfoReply {
        server_version: crate::VERSION.into(),
        features: features
            .into_iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name.to_owned())
            .collect(),
        clipboard_max_bytes: clipboard_max_bytes.unwrap_or(0),
        server_id: s.server_id.to_string(),
        fs_max_bytes: fs_max_bytes.unwrap_or(0),
        update_channel: s.update_channel.as_str().into(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::json::Fields;
    use super::*;

    #[test]
    fn a_message_goes_to_json_and_back() {
        let info = InfoReply {
            server_version: "1.2.3".into(),
            features: vec!["files".into(), "exec".into()],
            clipboard_max_bytes: 1 << 40,
            server_id: "id".into(),
            fs_max_bytes: 7,
            update_channel: "beta".into(),
        };
        let value = info.to_json();
        assert_eq!(value["clipboardMaxBytes"], json!("1099511627776"));
        assert_eq!(value["features"], json!(["files", "exec"]));
        assert_eq!(InfoReply::from_json(&value).unwrap(), info);
        let read = InfoReply::from_json(&json!({"fs_max_bytes": 3, "serverId": "x"})).unwrap();
        assert_eq!((read.fs_max_bytes, read.server_id.as_str()), (3, "x"));
        assert_eq!(
            InfoRequest::from_json(&Value::Null).unwrap(),
            InfoRequest {}
        );
        let _: Fields<'_> = Fields::of(&Value::Null).unwrap();
    }
}
