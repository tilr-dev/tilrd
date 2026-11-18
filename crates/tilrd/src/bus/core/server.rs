//! `server.*` (T2): `server.info`, `server.reload`, `server.update`,
//! `server.command`.
//!
//! Owner: `bus-core-services`.
//!
//! `server.update` and `server.command` run the server's own executable as an
//! exec (`exec::Execs`, as `tilr.v1`'s `ExecStart.server_command` did) and
//! stream it as `exec.run` does (`super::exec::stream_exec`). `server.update`
//! is `tilrd update`, which reloads the server once it installed a release.

use tilr_bus::bus::Lane;
use tilr_bus::server::{
    CommandRequest, InfoReply, InfoRequest, ReloadReply, ReloadRequest, UpdateRequest,
};
use tilr_bus::{BusError, methods};

use super::common::{asked, json_message, queue, unary};
use super::json;
use super::{Core, Registry};
use crate::bus::authz::{AuthzRequest, ForCall};
use crate::bus::identity::{CallCtx, Identity, Transport};
use crate::bus::streams::NativeMethod;
use crate::exec::Program;
use crate::supervisor::{
    Event, FEATURE_AGENT, FEATURE_CLIPBOARD, FEATURE_EXEC, FEATURE_FILES, FEATURE_HOST_EVENTS,
    Supervisor,
};

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
// server.*
// ---------------------------------------------------------------------------

/// Add `server.*`; `server.update` and `server.command` run even with the
/// `exec` feature off, as `tilr.v1`'s server commands do.
pub fn register(registry: &mut Registry, core: &Core) {
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

/// `server.reload` and `server.update`: the owner, or the `control` action of
/// the same name (`server.reload`, `server.update`).
fn may_reload(broker: &crate::bus::BrokerHandle, ctx: &CallCtx) -> Result<(), BusError> {
    tracing::info!(target: "tilrd::bus::audit", identity = %ctx.from, method = ctx.method, "control");
    broker
        .authorizer()
        .authorize(
            &ctx.principal,
            ctx.call_credential.as_deref(),
            &AuthzRequest::Control {
                action: &ctx.method,
                instance: None,
                call: ForCall {
                    method: &ctx.method,
                    target: &ctx.target,
                },
            },
        )
        .map_err(BusError::from)
}

/// The `server.info` feature of a server that enforces a profile's `[sandbox]`.
const FEATURE_SANDBOX: &str = "sandbox";

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
        // Sandboxed profiles start (S9): the start-up check passed.
        (FEATURE_SANDBOX, crate::sandbox::features().is_ok()),
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

    use super::*;
    use crate::bus::core::json::{Fields, Json};

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
