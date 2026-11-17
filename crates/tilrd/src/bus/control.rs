//! The `bus.*` control methods (B10): `bus.instances`, `bus.methods`, `bus.start`,
//! `bus.stop`, `bus.reload`, `bus.reload_config`.
//!
//! Owner: `bus-instances`. The listings read the runner's table at any time; what changes
//! something goes through the runner's one queue ([`Runner`]). Each method checks the
//! caller's `control` right itself (`AuthzRequest::Control`, action `list`, `start`,
//! `stop`, `reload` or `reload_config`, with the instance): the owner holds every right.
//! A caller that stops or reloads itself gets its reply first; the action follows after
//! [`REPLY_GRACE`].
//!
//! Requests and replies are protobuf (`tilr.bus.v1`, `methods.proto`) or proto3 JSON, as
//! the caller's `content_type` says.

use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use prost::Message;
use serde_json::{Value, json};
use tilr_bus::bus::{
    InstanceInfo, InstancesReply, Kind, Lane, MethodInfo, MethodsReply, ParticipantInfo,
    ReloadConfigReply, ReloadReply, Reloaded, Select, StartReply, StartRequest, StopReply,
    StopRequest, reload_request::What,
};
use tilr_bus::{BusError, CONTENT_TYPE_JSON, Reply, methods};

use super::authz::AuthzRequest;
use super::broker::BrokerHandle;
use super::config::MAX_DRAIN_MS;
use super::core::{Core, Registry};
use super::identity::{CallCtx, Identity, Transport};
use super::runner::{self, REPLY_GRACE, Runner, Target};
use super::streams::NativeMethod;

/// Add the control methods and start the runner (it reads `bus.toml` and starts the
/// instances).
pub fn register(registry: &mut Registry, core: &Core) {
    let runner = runner::start(core);
    let broker = core.broker.clone();
    macro_rules! method {
        ($name:expr, $handler:ident) => {{
            let runner = runner.clone();
            let broker = broker.clone();
            registry.add(NativeMethod::unary($name.name, move |ctx, payload| {
                let runner = runner.clone();
                let broker = broker.clone();
                async move { $handler(&runner, &broker, ctx, payload).await }
            }));
        }};
    }
    method!(methods::BUS_INSTANCES, instances);
    method!(methods::BUS_METHODS, method_list);
    method!(methods::BUS_START, start);
    method!(methods::BUS_STOP, stop);
    method!(methods::BUS_RELOAD, reload);
    method!(methods::BUS_RELOAD_CONFIG, reload_config);
}

// ---------------------------------------------------------------------------
// Payloads
// ---------------------------------------------------------------------------

fn request_json(method: &str, payload: &Bytes) -> Result<Value, BusError> {
    if payload.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(payload).map_err(|err| BusError::bad_request(format!("{method}: {err}")))
}

fn request_proto<M: Message + Default>(method: &str, payload: Bytes) -> Result<M, BusError> {
    M::decode(payload).map_err(|err| BusError::bad_request(format!("{method}: {err}")))
}

/// A string field of a JSON request, under its proto name or its camelCase one.
fn json_string(value: &Value, snake: &str, camel: &str) -> Option<String> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn json_u64(value: &Value, snake: &str, camel: &str) -> Result<Option<u64>, BusError> {
    let bad = || BusError::bad_request(format!("{snake} is not a number"));
    match value.get(snake).or_else(|| value.get(camel)) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number.as_u64().map(Some).ok_or_else(bad),
        Some(Value::String(text)) => text.parse().map(Some).map_err(|_| bad()),
        Some(_) => Err(bad()),
    }
}

fn reply<M: Message>(ctx: &CallCtx, message: &M, json: impl FnOnce() -> Value) -> Reply {
    if ctx.wants_json() {
        Reply::new(json().to_string()).with_content_type(CONTENT_TYPE_JSON)
    } else {
        Reply::encode(message)
    }
}

fn allow(
    broker: &BrokerHandle,
    ctx: &CallCtx,
    action: &str,
    instance: Option<&str>,
) -> Result<(), BusError> {
    tracing::info!(target: "tilrd::bus::audit", identity = %ctx.from, action, instance = instance.unwrap_or(""), "control");
    broker
        .authorizer()
        .authorize(
            &ctx.principal,
            ctx.call_credential.as_deref(),
            &AuthzRequest::Control { action, instance },
        )
        .map_err(BusError::from)
}

/// Whether `ctx` is the instance `name` itself.
fn is_caller(ctx: &CallCtx, name: &str) -> bool {
    matches!(&ctx.from, Identity::Instance(caller) if caller == name)
}

fn transport_name(transport: Transport) -> &'static str {
    match transport {
        Transport::UnixSocket => "unix",
        Transport::Tcp => "tcp",
        Transport::InstanceFd => "fd",
        Transport::InstanceSocket => "socket",
        // `Transport` grows with later changes; a new one is listed as such until named here.
        #[allow(
            unreachable_patterns,
            reason = "Transport is extended by other changes of the bus"
        )]
        _ => "other",
    }
}

// ---------------------------------------------------------------------------
// Listings
// ---------------------------------------------------------------------------

async fn instances(
    runner: &Runner,
    broker: &BrokerHandle,
    ctx: CallCtx,
    _payload: Bytes,
) -> Result<Reply, BusError> {
    allow(broker, &ctx, "list", None)?;
    let connections = broker.connections();
    let provides = |id: u64| -> Vec<String> {
        connections
            .iter()
            .find(|conn| conn.connection_id == id)
            .map(|conn| {
                conn.provides
                    .iter()
                    .map(|spec| spec.method.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    let message = InstancesReply {
        instances: runner
            .rows()
            .into_iter()
            .map(|row| InstanceInfo {
                provides: row.connection.map(provides).unwrap_or_default(),
                name: row.name,
                module: row.module,
                transport: row.transport.to_owned(),
                state: row.state.as_str().to_owned(),
                pid: row.pid,
                restarts: row.restarts,
            })
            .collect(),
        participants: connections
            .iter()
            .filter(|conn| !conn.identity.starts_with("instance:"))
            .map(|conn| ParticipantInfo {
                identity: conn.identity.clone(),
                client_name: conn.client_name.clone(),
                client_id: conn.client_id.clone(),
                connection_id: conn.connection_id,
                address: transport_name(conn.transport).to_owned(),
                origin: broker
                    .principal_of(conn.connection_id)
                    .map(|principal| principal.origin.clone())
                    .unwrap_or_default(),
                provides: provides(conn.connection_id),
            })
            .collect(),
    };
    Ok(reply(&ctx, &message, || {
        json!({
            "instances": message.instances.iter().map(|info| json!({
                "name": info.name,
                "module": info.module,
                "transport": info.transport,
                "state": info.state,
                "pid": info.pid,
                "restarts": info.restarts,
                "provides": info.provides,
            })).collect::<Vec<_>>(),
            "participants": message.participants.iter().map(|info| json!({
                "identity": info.identity,
                "clientName": info.client_name,
                "clientId": info.client_id,
                "connectionId": info.connection_id.to_string(),
                "address": info.address,
                "origin": info.origin,
                "provides": info.provides,
            })).collect::<Vec<_>>(),
        })
    }))
}

async fn method_list(
    _runner: &Runner,
    broker: &BrokerHandle,
    ctx: CallCtx,
    _payload: Bytes,
) -> Result<Reply, BusError> {
    allow(broker, &ctx, "list", None)?;
    let mut by_name: BTreeMap<String, MethodInfo> = BTreeMap::new();
    let mut add = |spec: &tilr_bus::bus::MethodSpec, provider: &str| {
        by_name
            .entry(spec.method.clone())
            .or_insert_with(|| MethodInfo {
                method: spec.method.clone(),
                kind: spec.kind,
                lane: spec.lane,
                select: spec.select,
                providers: Vec::new(),
            })
            .providers
            .push(provider.to_owned());
    };
    for spec in broker.native_methods() {
        add(&spec, &Identity::Tilrd.to_string());
    }
    // A draining provider gets no new calls by method: it is not offered.
    for conn in broker.connections().iter().filter(|conn| !conn.draining) {
        for spec in &conn.provides {
            add(spec, &conn.identity);
        }
    }
    let message = MethodsReply {
        methods: by_name.into_values().collect(),
    };
    Ok(reply(&ctx, &message, || {
        json!({
            "methods": message.methods.iter().map(|info| json!({
                "method": info.method,
                "kind": Kind::try_from(info.kind).map_or("UNARY", |kind| kind.as_str_name()),
                "lane": Lane::try_from(info.lane).map_or("INTERACTIVE", |lane| lane.as_str_name()),
                "select": Select::try_from(info.select).map_or("ROUND_ROBIN", |select| select.as_str_name()),
                "providers": info.providers,
            })).collect::<Vec<_>>(),
        })
    }))
}

// ---------------------------------------------------------------------------
// Changes
// ---------------------------------------------------------------------------

async fn start(
    runner: &Runner,
    broker: &BrokerHandle,
    ctx: CallCtx,
    payload: Bytes,
) -> Result<Reply, BusError> {
    let method = methods::BUS_START.name;
    let instance = if ctx.wants_json() {
        json_string(&request_json(method, &payload)?, "instance", "instance")
            .ok_or_else(|| BusError::bad_request(format!("{method}: name an instance")))?
    } else {
        request_proto::<StartRequest>(method, payload)?.instance
    };
    allow(broker, &ctx, "start", Some(&instance))?;
    let pid = runner.start(instance).await?;
    let message = StartReply { pid };
    Ok(reply(&ctx, &message, || json!({ "pid": pid })))
}

async fn stop(
    runner: &Runner,
    broker: &BrokerHandle,
    ctx: CallCtx,
    payload: Bytes,
) -> Result<Reply, BusError> {
    let method = methods::BUS_STOP.name;
    let (instance, drain_ms) = if ctx.wants_json() {
        let value = request_json(method, &payload)?;
        (
            json_string(&value, "instance", "instance")
                .ok_or_else(|| BusError::bad_request(format!("{method}: name an instance")))?,
            json_u64(&value, "drain_ms", "drainMs")?,
        )
    } else {
        let request = request_proto::<StopRequest>(method, payload)?;
        (request.instance, request.drain_ms.map(u64::from))
    };
    allow(broker, &ctx, "stop", Some(&instance))?;
    if let Some(drain_ms) = drain_ms.filter(|ms| *ms > MAX_DRAIN_MS) {
        return Err(BusError::bad_request(format!(
            "{method}: drain_ms = {drain_ms} is over the limit of {MAX_DRAIN_MS}"
        )));
    }
    let drain = drain_ms.map(Duration::from_millis);
    if is_caller(&ctx, &instance) {
        // The reply goes out first; the instance is stopped after it.
        let stopping = runner.stop(instance.clone(), drain, REPLY_GRACE);
        tokio::spawn(async move {
            if let Err(err) = stopping.await {
                tracing::error!(target: "tilrd::bus", "stop of {instance}: {err}");
            }
        });
    } else {
        runner.stop(instance, drain, Duration::ZERO).await?;
    }
    Ok(reply(&ctx, &StopReply {}, || json!({})))
}

async fn reload(
    runner: &Runner,
    broker: &BrokerHandle,
    ctx: CallCtx,
    payload: Bytes,
) -> Result<Reply, BusError> {
    let method = methods::BUS_RELOAD.name;
    let what = if ctx.wants_json() {
        let value = request_json(method, &payload)?;
        if let Some(instance) = json_string(&value, "instance", "instance") {
            Some(Target::Instance(instance))
        } else if let Some(module) = json_string(&value, "module", "module") {
            Some(Target::Module(module))
        } else {
            value
                .get("all")
                .and_then(Value::as_bool)
                .filter(|all| *all)
                .map(|_| Target::All)
        }
    } else {
        request_proto::<tilr_bus::bus::ReloadRequest>(method, payload)?
            .what
            .and_then(|what| match what {
                What::Instance(instance) => Some(Target::Instance(instance)),
                What::Module(module) => Some(Target::Module(module)),
                What::All(true) => Some(Target::All),
                What::All(false) => None,
            })
    };
    let what = what.ok_or_else(|| {
        BusError::bad_request(format!("{method}: name an instance, a module or all"))
    })?;
    let names = match &what {
        // Named: authorized before it is looked up.
        Target::Instance(name) => {
            allow(broker, &ctx, "reload", Some(name))?;
            runner.resolve(&what)?
        }
        // A module or all: the right over every instance first, so a caller without it
        // learns nothing of which modules there are or what runs.
        _ => {
            allow(broker, &ctx, "reload", None)?;
            let names = runner.resolve(&what)?;
            for name in &names {
                allow(broker, &ctx, "reload", Some(name))?;
            }
            names
        }
    };
    let reloaded = if names.iter().any(|name| is_caller(&ctx, name)) {
        // The caller is among them: it has its reply first, with the pids not yet known.
        let rows = runner.rows();
        let listed = names
            .iter()
            .map(|name| Reloaded {
                instance: name.clone(),
                old_pid: rows
                    .iter()
                    .find(|row| row.name == *name)
                    .map_or(0, |row| row.pid),
                new_pid: 0,
            })
            .collect();
        let reloading = runner.reload(names, REPLY_GRACE);
        tokio::spawn(async move {
            if let Err(err) = reloading.await {
                tracing::error!(target: "tilrd::bus", "reload: {err}");
            }
        });
        listed
    } else {
        runner
            .reload(names, Duration::ZERO)
            .await?
            .into_iter()
            .map(|done| Reloaded {
                instance: done.instance,
                old_pid: done.old_pid,
                new_pid: done.new_pid,
            })
            .collect()
    };
    let message = ReloadReply { reloaded };
    Ok(reply(&ctx, &message, || {
        json!({
            "reloaded": message.reloaded.iter().map(|done| json!({
                "instance": done.instance,
                "oldPid": done.old_pid,
                "newPid": done.new_pid,
            })).collect::<Vec<_>>(),
        })
    }))
}

async fn reload_config(
    runner: &Runner,
    broker: &BrokerHandle,
    ctx: CallCtx,
    _payload: Bytes,
) -> Result<Reply, BusError> {
    allow(broker, &ctx, "reload_config", None)?;
    let done = runner.reload_config().await?;
    let message = ReloadConfigReply {
        started: done.started,
        stopped: done.stopped,
        reloaded: done.reloaded,
        kept: done.kept,
    };
    Ok(reply(&ctx, &message, || {
        json!({
            "started": message.started,
            "stopped": message.stopped,
            "reloaded": message.reloaded,
            "kept": message.kept,
        })
    }))
}
