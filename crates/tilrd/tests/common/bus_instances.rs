//! Helpers of `bus-instances` for the bus tests, beside `bus.rs`: `bus.toml` text,
//! the `bus.*` control calls, waiting for what an instance does. Every wait is woken
//! by an event (a line of the server's log, a process's exit) and bounded by
//! [`TIMEOUT`]; nothing sleeps or polls.
//!
//! Owner: `bus-instances`.

use std::future::Future;
use std::path::{Path, PathBuf};

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

/// `name` as a TOML key: quoted when it holds a dot (an instance `a.b`), so it is one
/// key and not a nested table.
fn toml_key(name: &str) -> String {
    if name.contains('.') {
        toml_string(name)
    } else {
        name.to_owned()
    }
}

fn toml_array(items: &[&str]) -> String {
    let items: Vec<String> = items.iter().map(|item| toml_string(item)).collect();
    format!("[{}]", items.join(", "))
}

/// `[modules.<name>]` running `command`, with `env`.
pub fn module(name: &str, command: &[&str], env: &[(&str, &str)]) -> String {
    let name = toml_key(name);
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
    let name = toml_key(name);
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

/// `future`, failing the test when it is not done within [`TIMEOUT`].
pub async fn bounded<F: Future>(what: &str, future: F) -> F::Output {
    tokio::time::timeout(TIMEOUT, future)
        .await
        .unwrap_or_else(|_| panic!("{what}: not done within {TIMEOUT:?}"))
}

/// Wait until the process `pid` has exited (it may not be a child of the test), woken
/// by its exit: a pidfd on Linux, a kqueue `NOTE_EXIT` on macOS.
pub async fn wait_gone(pid: u32, what: &str) {
    let what = format!("{what} ({pid}) lives");
    let exited = tokio::task::spawn_blocking(move || exited_within(pid, TIMEOUT))
        .await
        .unwrap();
    assert!(exited, "{what}");
}

#[cfg(target_os = "linux")]
fn exited_within(pid: u32, wait: std::time::Duration) -> bool {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ESRCH),
            "pidfd_open {pid}: {err}"
        );
        return true;
    }
    // SAFETY: the descriptor was just opened and is owned here alone.
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
    loop {
        // SAFETY: one valid pollfd.
        match unsafe { libc::poll(&mut poll, 1, ms) } {
            1 => return true,
            0 => return false,
            _ => {
                let err = std::io::Error::last_os_error();
                assert_eq!(err.kind(), std::io::ErrorKind::Interrupted, "poll: {err}");
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn exited_within(pid: u32, wait: std::time::Duration) -> bool {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: kqueue takes nothing and returns a new descriptor or -1.
    let kq = unsafe { libc::kqueue() };
    assert!(kq >= 0, "kqueue: {}", std::io::Error::last_os_error());
    // SAFETY: the descriptor was just opened and is owned here alone.
    let kq = unsafe { OwnedFd::from_raw_fd(kq) };
    let change = libc::kevent {
        ident: pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    let timeout = libc::timespec {
        tv_sec: wait.as_secs() as libc::time_t,
        tv_nsec: libc::c_long::from(wait.subsec_nanos() as i32),
    };
    loop {
        // SAFETY: an all-zero kevent is a valid value to be written over.
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        // SAFETY: one change and room for one event, both valid for the call.
        let n = unsafe { libc::kevent(kq.as_raw_fd(), &change, 1, &mut event, 1, &timeout) };
        // Copied out: the struct is packed on some targets.
        let (flags, fflags, data) = (event.flags, event.fflags, event.data);
        match n {
            // A process that is gone (or a zombie) cannot be watched: it has exited.
            1 if flags & libc::EV_ERROR != 0 => {
                assert_eq!(data, libc::ESRCH as isize, "kevent {pid}");
                return true;
            }
            1 => return fflags & libc::NOTE_EXIT != 0,
            0 => return false,
            _ => {
                let err = std::io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(libc::ESRCH) => return true,
                    Some(libc::EINTR) => continue,
                    _ => panic!("kevent {pid}: {err}"),
                }
            }
        }
    }
}

/// `bus.instances`.
pub async fn listing(bus: &Bus) -> InstancesReply {
    bounded(
        "bus.instances",
        bus.call_typed(
            "bus.instances",
            &InstancesRequest {},
            CallOptions::default(),
        ),
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

/// How many lines the server logged about the instance `name` (`instance <name> ...`):
/// the runner logs each change of its listing after it made it.
fn instance_lines(lines: &[String], name: &str) -> usize {
    let (spaced, colon) = (format!("instance {name} "), format!("instance {name}:"));
    lines
        .iter()
        .filter(|line| line.contains(&spaced) || line.contains(&colon))
        .count()
}

/// Wait until `name` satisfies `done`: the listing is read again on each line the server
/// logs about `name`; fails with the last listing and the log after [`TIMEOUT`].
pub async fn wait_instance(
    server: &BusServer,
    bus: &Bus,
    name: &str,
    what: &str,
    done: impl Fn(&InstanceInfo) -> bool,
) -> InstanceInfo {
    bounded(&format!("{name}: {what}"), async {
        loop {
            // Counted before the listing is read, so a change in between is not missed.
            let seen = instance_lines(&server.log_lines(), name);
            let reply = listing(bus).await;
            if let Some(found) = reply.instances.iter().find(|info| info.name == name)
                && done(found)
            {
                return found.clone();
            }
            server
                .wait_logged_event(
                    &format!(
                        "{name}: {what} did not happen; listing: {:?}",
                        reply.instances
                    ),
                    |lines| instance_lines(lines, name) > seen,
                )
                .await;
        }
    })
    .await
}

/// Wait until `name` is `up`.
pub async fn wait_up(server: &BusServer, bus: &Bus, name: &str) -> InstanceInfo {
    wait_instance(server, bus, name, "coming up", |info| info.state == "up").await
}

/// `bus.methods`.
pub async fn method_list(bus: &Bus) -> MethodsReply {
    bounded(
        "bus.methods",
        bus.call_typed("bus.methods", &MethodsRequest {}, CallOptions::default()),
    )
    .await
    .expect("bus.methods")
}

pub async fn start(bus: &Bus, instance: &str) -> Result<StartReply, BusError> {
    bounded(
        "bus.start",
        bus.call_typed(
            "bus.start",
            &StartRequest {
                instance: instance.into(),
            },
            CallOptions::default(),
        ),
    )
    .await
}

pub async fn stop(bus: &Bus, instance: &str, drain_ms: Option<u32>) -> Result<StopReply, BusError> {
    bounded(
        "bus.stop",
        bus.call_typed(
            "bus.stop",
            &StopRequest {
                instance: instance.into(),
                drain_ms,
            },
            CallOptions::default(),
        ),
    )
    .await
}

pub async fn reload(bus: &Bus, what: What) -> Result<ReloadReply, BusError> {
    bounded(
        "bus.reload",
        bus.call_typed(
            "bus.reload",
            &ReloadRequest { what: Some(what) },
            CallOptions::default(),
        ),
    )
    .await
}

pub async fn reload_config(bus: &Bus) -> Result<ReloadConfigReply, BusError> {
    bounded(
        "bus.reload_config",
        bus.call_typed(
            "bus.reload_config",
            &ReloadConfigRequest {},
            CallOptions::default(),
        ),
    )
    .await
}

/// What a script instance writes to its standard error once it has written a line to
/// its file: the server logs it as `bus <instance>: written`.
pub const WRITTEN: &str = "echo written >&2";

/// The lines a script instance wrote to `file`, one per start.
pub fn tokens(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Wait until the script instance `instance` has written `count` lines to `file` (each
/// followed by [`WRITTEN`], which the server logs); returns them.
pub async fn wait_tokens(
    server: &BusServer,
    instance: &str,
    file: &Path,
    count: usize,
) -> Vec<String> {
    let line = format!("bus {instance}: written");
    server
        .wait_logged_event(
            &format!(
                "{instance} wrote fewer than {count} line(s) to {}",
                file.display()
            ),
            |lines| lines.iter().filter(|l| l.ends_with(&line)).count() >= count,
        )
        .await;
    let found = tokens(file);
    assert!(found.len() >= count, "{}: {found:?}", file.display());
    found
}

/// A module whose process only hands its socket token out (to `file`, one line per
/// start) and waits: the test connects to the instance socket as the instance.
pub fn token_module(name: &str, file: &Path) -> String {
    script_module(
        name,
        &format!("echo \"$TILR_BUS_TOKEN\" >> \"$TOKEN_FILE\"; {WRITTEN}; exec sleep 3600"),
        &[("TOKEN_FILE", file.to_str().expect("a UTF-8 path"))],
    )
}
