//! The test runtime: a bus participant for the tests of the broker, the instances and
//! the SDKs. Not part of a release (`scripts/release.sh` packs `tilrd` and `tilr` only).
//!
//! It connects as the environment says (`TILR_BUS_FD`; `TILR_BUS_SOCK` with
//! `TILR_BUS_TOKEN`; `TILR_BUS_URL`, `unix:<path>` for the server's Unix socket, with
//! `TILR_BUS_CREDENTIAL`) and provides, payloads as text:
//!
//! | method | answers |
//! |---|---|
//! | `test.echo` | the payload and `content_type` unchanged |
//! | `test.whoami` | its identity; the owner's connections answer their client name |
//! | `test.from` | the caller's identity (`from`) |
//! | `test.env` | JSON `{"env": {...}, "fds": [...]}`: its environment and the descriptors open when it started |
//! | `test.sleep` | `slept` after `<ms>` (default 10000); an abort is recorded |
//! | `test.started` | `started` once the handler of call `<id>` of `test.sleep`, `test.count`, `test.flood` or `test.bulk` has started, so a cancel sent after it finds it running |
//! | `test.nested` | `<wait_ms> <method> [<target>]`: waits, then calls `method` with the time left and answers its reply |
//! | `test.deadline` | the `timeout_ms` it was called with |
//! | `test.close_bus` | `closing`, then closes its bus connection and keeps running |
//! | `test.record` | records the payload, answers how many are recorded |
//! | `test.records` | every record, one per line, in order, once there are at least `<n>` (default 0) |
//! | `test.count` | stream: chunks `1` .. `<n>`, then `End`; a cancel is recorded |
//! | `test.flood` | stream (interactive): `<bytes>` (default: until cancelled) in chunks of 16 KiB, waiting for credit |
//! | `test.bulk` | stream (bulk): `<bytes>` (default 64 MiB) in chunks of 64 KiB, waiting for credit |
//! | `test.fail` | the error `<code> <message>` (default `bad_request x`) |
//! | `test.crash` | a panic: `handler_error` |
//! | `test.emit` | emits `<subject> <payload>` |
//! | `test.subscribe` | subscribes to `<pattern>`; each event is recorded as `event <subject> <from> <payload>` |
//!
//! Environment: `TEST_CLIENT_NAME` (else `TILR_INSTANCE`, else `test-runtime`) is the
//! client name; `TEST_SUBSCRIBE` names patterns (comma-separated) to subscribe to at
//! start; `TEST_NO_HELLO` set: connect (or keep descriptor 3) and never say `Hello`;
//! `TEST_STDERR` is written to stderr at start. It exits 0 when its connection ends,
//! unless `test.close_bus` closed it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use tilr_bus::bus::{Lane, MethodSpec};
use tilr_bus::{
    Bus, BusError, CallOptions, ConnectOptions, Handler, Reply, Request, StreamWriter, Transport,
    codes,
};
use tokio::sync::watch;

static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The calls whose handler has started (`test.started`).
static STARTED: Mutex<Vec<u64>> = Mutex::new(Vec::new());
/// Bumped whenever RECORDS or STARTED changes, for the methods that wait for them.
static CHANGED: LazyLock<watch::Sender<()>> = LazyLock::new(|| watch::Sender::new(()));
static FDS_AT_START: OnceLock<Vec<i32>> = OnceLock::new();
static CLIENT_NAME: OnceLock<String> = OnceLock::new();
static CLOSING: AtomicBool = AtomicBool::new(false);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn record(line: impl Into<String>) -> usize {
    let len = {
        let mut records = lock(&RECORDS);
        records.push(line.into());
        records.len()
    };
    CHANGED.send_modify(|_| {});
    len
}

/// Wait until `ready` holds, woken by every change of RECORDS or STARTED.
async fn wait_until(ready: impl Fn() -> bool) {
    let mut changed = CHANGED.subscribe();
    while !ready() {
        if changed.changed().await.is_err() {
            return;
        }
    }
}

/// Records `what` unless disarmed: a handler aborted by a cancel never disarms it.
struct OnAbort(Option<String>);

impl OnAbort {
    /// Arm the guard of call `id`'s handler, which has now started.
    fn start(id: u64, what: String) -> Self {
        lock(&STARTED).push(id);
        CHANGED.send_modify(|_| {});
        OnAbort(Some(what))
    }

    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for OnAbort {
    fn drop(&mut self) {
        if let Some(what) = self.0.take() {
            record(what);
        }
    }
}

fn text(request: &Request) -> String {
    String::from_utf8_lossy(&request.payload).trim().to_owned()
}

fn number<T: std::str::FromStr>(request: &Request, default: T) -> Result<T, BusError> {
    let text = text(request);
    if text.is_empty() {
        return Ok(default);
    }
    text.parse()
        .map_err(|_| BusError::bad_request(format!("{}: {text:?} is not a number", request.method)))
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The descriptors open now, probed without opening any.
fn open_fds() -> Vec<i32> {
    (0..1024)
        .filter(|fd| std::fs::metadata(format!("/dev/fd/{fd}")).is_ok())
        .collect()
}

async fn env(_: Request) -> Result<String, BusError> {
    let mut vars: Vec<(String, String)> = std::env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().into_owned(),
                v.to_string_lossy().into_owned(),
            )
        })
        .collect();
    vars.sort();
    let vars: Vec<String> = vars
        .iter()
        .map(|(k, v)| format!("{}:{}", json_string(k), json_string(v)))
        .collect();
    let fds: Vec<String> = FDS_AT_START
        .get()
        .map(|fds| fds.iter().map(i32::to_string).collect())
        .unwrap_or_default();
    Ok(format!(
        r#"{{"env":{{{}}},"fds":[{}]}}"#,
        vars.join(","),
        fds.join(",")
    ))
}

async fn sleep(request: Request) -> Result<&'static str, BusError> {
    let ms: u64 = number(&request, 10_000)?;
    let guard = OnAbort::start(request.id, format!("aborted test.sleep {}", request.id));
    tokio::time::sleep(Duration::from_millis(ms)).await;
    guard.disarm();
    Ok("slept")
}

async fn nested(request: Request) -> Result<Reply, BusError> {
    let text = text(&request);
    let mut parts = text.split_whitespace();
    let (Some(wait), Some(method)) = (parts.next(), parts.next()) else {
        return Err(BusError::bad_request(
            "test.nested: <wait_ms> <method> [<target>]",
        ));
    };
    let wait: u64 = wait
        .parse()
        .map_err(|_| BusError::bad_request(format!("test.nested: {wait:?} is not a number")))?;
    tokio::time::sleep(Duration::from_millis(wait)).await;
    let mut options = CallOptions::default();
    if let Some(target) = parts.next() {
        options = options.target(target);
    }
    if let Some(left) = request.time_left() {
        options = options.timeout(left.max(Duration::from_millis(1)));
    }
    request.bus().call(method, Bytes::new(), options).await
}

async fn count(request: Request, out: StreamWriter) -> Result<Option<Bytes>, BusError> {
    let n: u64 = number(&request, 0)?;
    let guard = OnAbort::start(request.id, format!("cancelled test.count {}", request.id));
    for i in 1..=n {
        out.write(i.to_string()).await?;
    }
    guard.disarm();
    Ok(None)
}

async fn flood(
    request: Request,
    out: StreamWriter,
    default: u64,
    chunk: usize,
) -> Result<Option<Bytes>, BusError> {
    let total: u64 = number(&request, default)?;
    let guard = OnAbort::start(
        request.id,
        format!("cancelled {} {}", request.method, request.id),
    );
    let chunk = chunk.min(out.window() as usize).max(1);
    let block = Bytes::from(vec![b'x'; chunk]);
    let mut sent = 0u64;
    while sent < total {
        let len = (total - sent).min(chunk as u64) as usize;
        out.write(block.slice(..len)).await?;
        sent += len as u64;
    }
    guard.disarm();
    Ok(None)
}

async fn fail(request: Request) -> Result<Bytes, BusError> {
    let text = text(&request);
    let (code, message) = text.split_once(' ').unwrap_or((text.as_str(), ""));
    if code.is_empty() {
        return Err(BusError::bad_request("x"));
    }
    Err(BusError::new(code, message))
}

async fn crash(_: Request) -> Result<Bytes, BusError> {
    panic!("test.crash: a plain failure")
}

async fn emit(request: Request) -> Result<&'static str, BusError> {
    let text = text(&request);
    let (subject, payload) = text.split_once(' ').unwrap_or((text.as_str(), ""));
    request.bus().emit(subject, payload.to_owned()).await?;
    Ok("emitted")
}

async fn subscribe(bus: &Bus, pattern: &str) -> Result<(), BusError> {
    let mut subscription = bus.subscribe(pattern).await?;
    tokio::spawn(async move {
        while let Some(event) = subscription.next().await {
            record(format!(
                "event {} {} {}",
                event.subject,
                event.from,
                String::from_utf8_lossy(&event.payload)
            ));
        }
    });
    Ok(())
}

fn options(client_name: &str) -> ConnectOptions {
    let unary = MethodSpec::unary;
    ConnectOptions::new(client_name)
        .provide(
            unary("test.echo"),
            Handler::unary(|request: Request| async move {
                Ok(Reply::new(request.payload).with_content_type(request.content_type))
            }),
        )
        .provide(
            unary("test.whoami"),
            Handler::unary(|request: Request| async move {
                let identity = request.bus().identity();
                Ok(if identity == "owner" {
                    CLIENT_NAME.get().cloned().unwrap_or_default()
                } else {
                    identity.to_owned()
                })
            }),
        )
        .provide(
            unary("test.from"),
            Handler::unary(|request: Request| async move { Ok(request.from) }),
        )
        .provide(unary("test.env"), Handler::unary(env))
        .provide(unary("test.sleep"), Handler::unary(sleep))
        .provide(unary("test.nested"), Handler::unary(nested))
        .provide(
            unary("test.deadline"),
            Handler::unary(|request: Request| async move { Ok(request.timeout_ms.to_string()) }),
        )
        .provide(
            unary("test.close_bus"),
            Handler::unary(|request: Request| async move {
                let bus = request.bus().clone();
                tokio::spawn(async move {
                    // The reply goes out first.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    CLOSING.store(true, Ordering::SeqCst);
                    bus.close();
                });
                Ok("closing")
            }),
        )
        .provide(
            unary("test.record"),
            Handler::unary(|request: Request| async move {
                Ok(record(String::from_utf8_lossy(&request.payload)).to_string())
            }),
        )
        .provide(
            unary("test.records"),
            Handler::unary(|request: Request| async move {
                let at_least: usize = number(&request, 0)?;
                wait_until(|| lock(&RECORDS).len() >= at_least).await;
                Ok(lock(&RECORDS).join("\n"))
            }),
        )
        .provide(
            unary("test.started"),
            Handler::unary(|request: Request| async move {
                let id: u64 = number(&request, 0)?;
                wait_until(|| lock(&STARTED).contains(&id)).await;
                Ok("started")
            }),
        )
        .provide(
            MethodSpec::stream("test.count", Lane::Interactive),
            Handler::stream(count),
        )
        .provide(
            MethodSpec::stream("test.flood", Lane::Interactive),
            Handler::stream(|request, out| flood(request, out, u64::MAX, 16 << 10)),
        )
        .provide(
            MethodSpec::stream("test.bulk", Lane::Bulk),
            Handler::stream(|request, out| flood(request, out, 64 << 20, 64 << 10)),
        )
        .provide(unary("test.fail"), Handler::unary(fail))
        .provide(unary("test.crash"), Handler::unary(crash))
        .provide(unary("test.emit"), Handler::unary(emit))
        .provide(
            unary("test.subscribe"),
            Handler::unary(|request: Request| async move {
                subscribe(request.bus(), &text(&request)).await?;
                Ok("subscribed")
            }),
        )
}

fn hold_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn main() {
    let _ = FDS_AT_START.set(open_fds());
    let client_name = std::env::var("TEST_CLIENT_NAME")
        .or_else(|_| std::env::var(tilr_bus::env::INSTANCE))
        .unwrap_or_else(|_| "test-runtime".into());
    let _ = CLIENT_NAME.set(client_name.clone());
    if let Ok(line) = std::env::var("TEST_STDERR") {
        eprintln!("{line}");
    }
    if std::env::var_os("TEST_NO_HELLO").is_some() {
        // Descriptor 3 stays open as inherited; a socket is connected and kept.
        let _stream = std::env::var(tilr_bus::env::BUS_SOCK)
            .ok()
            .and_then(|path| std::os::unix::net::UnixStream::connect(path).ok());
        hold_forever();
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = runtime.block_on(async {
        let transport = match Transport::from_env() {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("test_runtime: {error}");
                return 2;
            }
        };
        let bus = match Bus::connect(transport, options(&client_name)).await {
            Ok(bus) => bus,
            Err(error) => {
                eprintln!("test_runtime: {error}");
                return if error.code == codes::UNAVAILABLE {
                    3
                } else {
                    4
                };
            }
        };
        for pattern in std::env::var("TEST_SUBSCRIBE")
            .unwrap_or_default()
            .split(',')
        {
            let pattern = pattern.trim();
            if !pattern.is_empty()
                && let Err(error) = subscribe(&bus, pattern).await
            {
                eprintln!("test_runtime: subscribe {pattern}: {error}");
            }
        }
        let info = bus.closed().await;
        if CLOSING.load(Ordering::SeqCst) {
            -1
        } else {
            eprintln!("test_runtime: connection ended: {info:?}");
            0
        }
    });
    if code < 0 {
        // test.close_bus: the connection is gone, the process stays.
        hold_forever();
    }
    std::process::exit(code);
}
