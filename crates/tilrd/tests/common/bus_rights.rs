//! Helpers of `bus-rights` (and `bus-pane-rights`) for the bus tests, beside `bus.rs`.
//!
//! Owner: `bus-rights`. Every token here is minted by the server under test during
//! the test (`bus.token`); none is written down.

use std::path::{Path, PathBuf};

use tilr_bus::proto::tilr::bus::v1 as wire;
use tilr_bus::{Bus, BusError, CallOptions, ConnectOptions, Credential, Transport};

use super::Server;
use super::bus::{BusOptions, BusServer, TestRuntime};

/// The rights of a token, as `bus.token` takes them.
#[derive(Clone, Debug, Default)]
pub struct Rights {
    pub call: Vec<String>,
    pub provide: Vec<String>,
    pub emit: Vec<String>,
    pub subscribe: Vec<String>,
    pub control: Vec<String>,
    pub pane: Vec<String>,
    pub origin: String,
    pub ttl_ms: u64,
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

impl Rights {
    pub fn call(mut self, list: &[&str]) -> Self {
        self.call = strings(list);
        self
    }
    pub fn provide(mut self, list: &[&str]) -> Self {
        self.provide = strings(list);
        self
    }
    pub fn emit(mut self, list: &[&str]) -> Self {
        self.emit = strings(list);
        self
    }
    pub fn subscribe(mut self, list: &[&str]) -> Self {
        self.subscribe = strings(list);
        self
    }
    pub fn control(mut self, list: &[&str]) -> Self {
        self.control = strings(list);
        self
    }
    pub fn ttl_ms(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = ttl_ms;
        self
    }
}

/// The test runtime's rights: it provides `test.*`.
pub fn runtime_rights() -> Rights {
    Rights::default().provide(&["test.*"])
}

/// `bus.token` for `token:<name>`, by `minter` (the owner).
pub async fn mint(minter: &Bus, name: &str, rights: Rights) -> (Credential, wire::TokenReply) {
    try_mint(minter, name, rights)
        .await
        .unwrap_or_else(|err| panic!("bus.token {name}: {err}"))
}

pub async fn try_mint(
    minter: &Bus,
    name: &str,
    rights: Rights,
) -> Result<(Credential, wire::TokenReply), BusError> {
    let reply: wire::TokenReply = minter
        .call_typed(
            "bus.token",
            &wire::TokenRequest {
                name: name.into(),
                call: rights.call,
                provide: rights.provide,
                emit: rights.emit,
                subscribe: rights.subscribe,
                control: rights.control,
                pane: rights.pane,
                origin: rights.origin,
                ttl_ms: rights.ttl_ms,
            },
            CallOptions::default(),
        )
        .await?;
    Ok((Credential::from_bytes(reply.credential.clone()), reply))
}

/// An outside participant on the Unix socket with `credential`.
pub async fn connect(
    server: &BusServer,
    credential: &Credential,
    options: ConnectOptions,
) -> Result<Bus, BusError> {
    Bus::connect(
        Transport::UnixWebSocket {
            path: server.socket(),
            credential: Some(credential.clone()),
            origin: None,
        },
        options,
    )
    .await
}

/// [`connect`] that must succeed.
pub async fn participant(server: &BusServer, credential: &Credential, name: &str) -> Bus {
    connect(server, credential, ConnectOptions::new(name))
        .await
        .unwrap_or_else(|err| panic!("{name} with its token: {err}"))
}

/// The test runtime as `token:<name>`, providing `test.*` with a token minted by
/// `owner` with `rights` (plus `provide = ["test.*"]`).
pub async fn runtime(server: &BusServer, owner: &Bus, name: &str, rights: Rights) -> TestRuntime {
    let rights = Rights {
        provide: vec!["test.*".into()],
        ..rights
    };
    let (credential, _) = mint(owner, name, rights).await;
    server
        .test_runtime(
            name,
            &[("TILR_BUS_CREDENTIAL", credential.to_base64url().as_str())],
        )
        .await
}

/// A call's reply as text, or its error.
pub async fn calls(
    bus: &Bus,
    method: &str,
    payload: &str,
    options: CallOptions,
) -> Result<String, BusError> {
    bus.call(method, payload.to_owned(), options)
        .await
        .map(|reply| String::from_utf8_lossy(&reply.payload).into_owned())
}

/// The server's `bus audit:` lines.
pub fn audit_lines(server: &BusServer) -> Vec<String> {
    server
        .log_lines()
        .into_iter()
        .filter(|line| line.contains("bus audit:"))
        .collect()
}

/// Wait until the server logged a line containing every one of `parts`, woken by
/// each line it logs and bounded by [`TIMEOUT`](super::TIMEOUT); that line.
pub async fn wait_logged(server: &BusServer, parts: &[&str]) -> String {
    let has = |line: &String| parts.iter().all(|part| line.contains(part));
    server
        .wait_logged_event(&format!("not logged: {parts:?}"), |lines| {
            lines.iter().any(has)
        })
        .await;
    server
        .log_lines()
        .into_iter()
        .find(has)
        .expect("the line just seen")
}

/// The server's state directory (after [`restart`]: the [`Scratch`]'s `state`).
pub fn state_dir(server: &BusServer) -> PathBuf {
    server.dir.join("state")
}

/// A scratch directory removed when dropped.
pub struct Scratch(pub PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Stop `server` and start another on a copy of its state directory (its id, its
/// root key, its revocations), with `options`: a restart. Keep the [`Scratch`] while
/// the new server runs.
pub fn restart(server: BusServer, options: BusOptions) -> (BusServer, Scratch) {
    let kept = Scratch(Server::scratch_dir());
    let state = kept.0.join("state");
    copy_dir(&state_dir(&server), &state);
    drop(server);
    let mut options = options;
    options
        .env
        .push(("TILR_STATE_DIR".into(), state.display().to_string()));
    (BusServer::start(options), kept)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            // `copy` keeps the mode.
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

/// A test clock (`TILR_TEST_TIME_FILE`, `failpoints` builds) in Unix microseconds.
#[cfg(feature = "failpoints")]
pub struct TestClock {
    pub path: PathBuf,
    _dir: Scratch,
}

#[cfg(feature = "failpoints")]
impl TestClock {
    pub fn new(start_us: u64) -> Self {
        let dir = Scratch(Server::scratch_dir());
        let clock = Self {
            path: dir.0.join("clock"),
            _dir: dir,
        };
        clock.set(start_us);
        clock
    }

    pub fn set(&self, us: u64) {
        super::time::set_test_time(&self.path, us);
    }

    /// The server's environment entry for it.
    pub fn env(&self) -> (String, String) {
        (
            "TILR_TEST_TIME_FILE".to_owned(),
            self.path.display().to_string(),
        )
    }
}

/// Microseconds in an hour.
pub const HOUR_US: u64 = 3600 * 1_000_000;
