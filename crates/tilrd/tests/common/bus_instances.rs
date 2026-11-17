//! Helpers of `bus-instances` for the bus tests, beside `bus.rs`: `bus.toml` text,
//! the `bus.*` control calls, waiting for what an instance does.
//!
//! Owner: `bus-instances`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tilr_bus::bus::{
    InstanceInfo, InstancesReply, InstancesRequest, MethodsReply, MethodsRequest,
    ReloadConfigReply, ReloadConfigRequest, ReloadReply, ReloadRequest, StartReply, StartRequest,
    StopReply, StopRequest, reload_request::What,
};
use tilr_bus::{Bus, BusError, CallOptions};

use super::TIMEOUT;
use super::bus::{BusServer, test_runtime_path};

/// A TOML basic string.
pub fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_array(items: &[&str]) -> String {
    let items: Vec<String> = items.iter().map(|item| toml_string(item)).collect();
    format!("[{}]", items.join(", "))
}

/// `[modules.<name>]` running `command`, with `env`.
pub fn module(name: &str, command: &[&str], env: &[(&str, &str)]) -> String {
    let mut text = format!("[modules.{name}]\ncommand = {}\n", toml_array(command));
    if !env.is_empty() {
        text.push_str(&format!("[modules.{name}.env]\n"));
        for (key, value) in env {
            text.push_str(&format!("{key} = {}\n", toml_string(value)));
        }
    }
    text
}

/// `[modules.<name>]` running the test runtime.
pub fn test_module(name: &str, env: &[(&str, &str)]) -> String {
    let path = test_runtime_path();
    module(name, &[path.to_str().expect("a UTF-8 path")], env)
}

/// A module that runs `script` with `/bin/sh -c`, with `env`.
pub fn script_module(name: &str, script: &str, env: &[(&str, &str)]) -> String {
    module(name, &["/bin/sh", "-c", script], env)
}

/// `[instances.<name>]` of `module`: `top` is more keys of the table (`connect = ...`,
/// `autostart = ...`, each ending in a newline), `rights` the lines of its `rights`
/// table, `env` its environment.
pub fn instance_with(
    name: &str,
    module: &str,
    top: &str,
    rights: &[(&str, &[&str])],
    env: &[(&str, &str)],
) -> String {
    let mut text = format!(
        "[instances.{name}]\nmodule = {}\n{top}",
        toml_string(module)
    );
    if !rights.is_empty() {
        text.push_str(&format!("[instances.{name}.rights]\n"));
        for (key, patterns) in rights {
            text.push_str(&format!("{key} = {}\n", toml_array(patterns)));
        }
    }
    if !env.is_empty() {
        text.push_str(&format!("[instances.{name}.env]\n"));
        for (key, value) in env {
            text.push_str(&format!("{key} = {}\n", toml_string(value)));
        }
    }
    text
}

/// An instance of the test runtime module that may provide `test.*`.
pub fn instance(name: &str, module: &str) -> String {
    instance_with(name, module, "", &[("provide", &["test.*"])], &[])
}

/// Replace `bus.toml` of the server (a rename, so a reader sees it whole).
pub fn write_bus_toml(server: &BusServer, text: &str) {
    let dir = server.dir.join("config");
    let next = dir.join("bus.toml.next");
    std::fs::write(&next, text).unwrap();
    std::fs::rename(next, dir.join("bus.toml")).unwrap();
}

/// The socket of a socket instance.
pub fn instance_socket(server: &BusServer, name: &str) -> PathBuf {
    server.dir.join("runtime/bus").join(format!("{name}.sock"))
}

/// Whether a process lives.
pub fn alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 only checks.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// `bus.instances`.
pub async fn listing(bus: &Bus) -> InstancesReply {
    bus.call_typed(
        "bus.instances",
        &InstancesRequest {},
        CallOptions::default(),
    )
    .await
    .expect("bus.instances")
}

/// One instance of the listing.
pub async fn info(bus: &Bus, name: &str) -> InstanceInfo {
    listing(bus)
        .await
        .instances
        .into_iter()
        .find(|info| info.name == name)
        .unwrap_or_else(|| panic!("{name} is not listed"))
}

/// Wait until `name` satisfies `done`; panics with the listing when [`TIMEOUT`] passes.
pub async fn wait_instance(
    bus: &Bus,
    name: &str,
    what: &str,
    done: impl Fn(&InstanceInfo) -> bool,
) -> InstanceInfo {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let reply = listing(bus).await;
        if let Some(found) = reply.instances.iter().find(|info| info.name == name)
            && done(found)
        {
            return found.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{name}: {what} did not happen; listing: {:?}",
            reply.instances
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wait until `name` is `up`.
pub async fn wait_up(bus: &Bus, name: &str) -> InstanceInfo {
    wait_instance(bus, name, "coming up", |info| info.state == "up").await
}

/// `bus.methods`.
pub async fn method_list(bus: &Bus) -> MethodsReply {
    bus.call_typed("bus.methods", &MethodsRequest {}, CallOptions::default())
        .await
        .expect("bus.methods")
}

pub async fn start(bus: &Bus, instance: &str) -> Result<StartReply, BusError> {
    bus.call_typed(
        "bus.start",
        &StartRequest {
            instance: instance.into(),
        },
        CallOptions::default(),
    )
    .await
}

pub async fn stop(bus: &Bus, instance: &str, drain_ms: Option<u32>) -> Result<StopReply, BusError> {
    bus.call_typed(
        "bus.stop",
        &StopRequest {
            instance: instance.into(),
            drain_ms,
        },
        CallOptions::default(),
    )
    .await
}

pub async fn reload(bus: &Bus, what: What) -> Result<ReloadReply, BusError> {
    bus.call_typed(
        "bus.reload",
        &ReloadRequest { what: Some(what) },
        CallOptions::default(),
    )
    .await
}

pub async fn reload_config(bus: &Bus) -> Result<ReloadConfigReply, BusError> {
    bus.call_typed(
        "bus.reload_config",
        &ReloadConfigRequest {},
        CallOptions::default(),
    )
    .await
}

/// The tokens a script instance wrote to `file`, one per start.
pub fn tokens(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Wait until `file` holds `count` tokens; returns them.
pub async fn wait_tokens(file: &Path, count: usize) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let found = tokens(file);
        if found.len() >= count {
            return found;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} holds {} token(s), not {count}",
            file.display(),
            found.len()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A module whose process only hands its socket token out (to `file`, one line per
/// start) and waits: the test connects to the instance socket as the instance.
pub fn token_module(name: &str, file: &Path) -> String {
    script_module(
        name,
        "echo \"$TILR_BUS_TOKEN\" >> \"$TOKEN_FILE\"; exec sleep 3600",
        &[("TOKEN_FILE", file.to_str().expect("a UTF-8 path"))],
    )
}
