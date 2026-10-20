//! The link to tilr Cloud. Optional: a server that was never linked does
//! nothing here.
//!
//! A linked server keeps one outgoing connection to Cloud, the control
//! connection. Over it the server says it is online, and Cloud says whether
//! the server should have a tunnel: a name on the internet that leads to
//! this server's WebSocket port, carried by a `cloudflared` this module
//! runs. Clients then reach the server through the tunnel, not through
//! Cloud.
//!
//! Cloud is not trusted with anything else. No message from it reaches a
//! pane; what it sends about a tunnel is checked before it is used; and
//! where the tunnel leads is written here, not there. Who may use the server
//! through the tunnel is decided by the access token (see [`crate::auth`]),
//! which Cloud never sees. Without the token check there is no tunnel.

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{self, Message};

use crate::{clipboard, identity};

pub const DEFAULT_URL: &str = "https://wss.tilr.dev";
/// Version of the control protocol, see SPEC.md of tilr Cloud.
const PROTOCOL: u32 = 1;
const LINK_FILE: &str = "cloud.json";

/// How often the link file is looked at for a login or a logout.
const LINK_CHECK: Duration = Duration::from_secs(3);
/// A ping goes out every this many link checks.
const CHECKS_PER_PING: u32 = 10;
/// Cloud is given up on, and connected to again, after this long without an
/// answer to a ping.
const SILENCE: Duration = Duration::from_secs(100);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const RETRY_MIN: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// Pause before a `cloudflared` that exited is started again.
const TUNNEL_RETRY: Duration = Duration::from_secs(5);
/// How long `cloudflared` gets to exit when asked to.
const TUNNEL_STOP: Duration = Duration::from_secs(3);

/// Close codes of the control connection.
const CLOSE_REVOKED: u16 = 4001;

/// What linking leaves on disk: where Cloud is and the token this machine
/// connects with.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Link {
    url: String,
    machine_id: String,
    token: String,
}

fn link_path() -> Result<PathBuf> {
    Ok(identity::state_dir()
        .context("no home directory")?
        .join(LINK_FILE))
}

fn read_link() -> Option<Link> {
    let text = fs::read_to_string(link_path().ok()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// When the link file last changed; `None` without one.
fn link_stamp() -> Option<SystemTime> {
    fs::metadata(link_path().ok()?).ok()?.modified().ok()
}

fn write_link(link: &Link) -> Result<()> {
    let path = link_path()?;
    let dir = path.parent().context("state directory")?;
    identity::create_private_dir(dir)?;
    // Written aside and renamed: a running server never reads half a file.
    let staged = dir.join(format!("{LINK_FILE}.{}", std::process::id()));
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)
        .and_then(|file| Ok(serde_json::to_writer_pretty(file, link)?))
        .and_then(|()| fs::rename(&staged, &path));
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written.with_context(|| format!("write {}", path.display()))
}

// ---------------------------------------------------------------------------
// tilrd cloud login | logout | status
// ---------------------------------------------------------------------------

fn post(url: &str, body: &Value) -> Result<Value, ureq::Error> {
    let text = ureq::post(url)
        .header("content-type", "application/json")
        .send(body.to_string())?
        .body_mut()
        .read_to_string()?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn host_name() -> String {
    let mut name = [0u8; 256];
    // SAFETY: the buffer is valid for the length given, minus one so the
    // name always ends in a NUL.
    let ok = unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len() - 1) } == 0;
    let len = name.iter().position(|&byte| byte == 0).unwrap_or(0);
    if ok {
        String::from_utf8_lossy(&name[..len]).into_owned()
    } else {
        String::new()
    }
}

/// Link this machine to a user of Cloud: show a code, wait until the user
/// confirms it in their browser, keep the token Cloud hands over.
pub fn login(url: &str, name: Option<String>) -> Result<()> {
    let url = url.trim_end_matches('/');
    let asked = post(
        &format!("{url}/v1/enroll"),
        &json!({
            "server_id": identity::server_id().to_string(),
            "name": name.unwrap_or_else(host_name),
            "platform": crate::PLATFORM,
            "version": crate::VERSION,
        }),
    )
    .with_context(|| format!("ask {url} to link this machine"))?;
    let text = |name: &str| asked[name].as_str().unwrap_or_default();
    let device_code = text("device_code");
    ensure!(
        !device_code.is_empty(),
        "{url} did not answer as tilr Cloud"
    );
    println!(
        "To link this machine, open\n\n    {}\n\nand confirm the code {}",
        text("verify_url"),
        text("user_code")
    );

    let interval = Duration::from_secs(asked["interval"].as_u64().unwrap_or(3).clamp(1, 30));
    let poll = json!({ "device_code": device_code });
    loop {
        std::thread::sleep(interval);
        let answer = match post(&format!("{url}/v1/enroll/poll"), &poll) {
            Ok(answer) => answer,
            Err(ureq::Error::StatusCode(404)) => bail!("the code expired, nothing was linked"),
            // The network may come back before the code expires.
            Err(_) => continue,
        };
        if answer["status"] != "approved" {
            continue;
        }
        let field = |name: &str| answer[name].as_str().map(str::to_owned);
        write_link(&Link {
            url: url.to_owned(),
            machine_id: field("machine_id").context("no machine id in the answer")?,
            token: field("machine_token").context("no token in the answer")?,
        })?;
        println!("\nLinked. A running tilrd connects on its own.");
        return Ok(());
    }
}

/// Unlink this machine: Cloud forgets it and takes its tunnel away.
pub fn logout() -> Result<()> {
    let Some(link) = read_link() else {
        println!("This machine is not linked.");
        return Ok(());
    };
    let told = ureq::delete(&format!("{}/v1/machine", link.url))
        .header("authorization", &format!("Bearer {}", link.token))
        .call();
    fs::remove_file(link_path()?).context("remove the link")?;
    match told {
        Ok(_) | Err(ureq::Error::StatusCode(401)) => println!("Unlinked."),
        Err(err) => println!(
            "Unlinked here, but {} could not be told ({err}): remove the machine there.",
            link.url
        ),
    }
    Ok(())
}

pub fn status() -> Result<()> {
    match read_link() {
        Some(link) => println!("Linked to {} as machine {}.", link.url, link.machine_id),
        None => println!("This machine is not linked. Link it with: tilr cloud login"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The control connection
// ---------------------------------------------------------------------------

pub struct Options {
    /// Clients must present the access token. Without that, no tunnel.
    pub auth: bool,
    /// Where the server listens: what a tunnel leads to.
    pub local: SocketAddr,
    /// The `cloudflared` to run.
    pub cloudflared: PathBuf,
}

/// The `cloudflared` of a running server, as seen from outside the task
/// that runs it.
#[derive(Clone)]
pub struct Tunnel {
    control: mpsc::UnboundedSender<Control>,
}

enum Control {
    /// Stop `cloudflared`, wait until it is gone, say so, and start none
    /// until `Release`.
    Hold(oneshot::Sender<()>),
    Release,
}

impl Tunnel {
    /// No tunnel: a server without a TCP port has nothing for one to lead to.
    pub fn off() -> Self {
        let (control, _) = mpsc::unbounded_channel();
        Self { control }
    }

    /// Stop `cloudflared` and wait until it has exited and been waited for,
    /// before a server replaces itself, which starts another: a child that
    /// exits after the exec stays a zombie of the successor, which knows
    /// nothing of it, and one that is slow to exit would run alongside the
    /// new one. None is started again until [`Tunnel::release`].
    pub async fn hold(&self) {
        let (done, stopped) = oneshot::channel();
        if self.control.send(Control::Hold(done)).is_ok() {
            // `stop` kills it after TUNNEL_STOP; this bound is for a task
            // that is somehow stuck, not for cloudflared.
            let _ = tokio::time::timeout(TUNNEL_STOP + Duration::from_secs(1), stopped).await;
        }
    }

    /// A server that carries on after all gets its `cloudflared` back.
    pub fn release(&self) {
        let _ = self.control.send(Control::Release);
    }
}

/// Keep this server connected to Cloud for as long as it is linked.
pub fn spawn(options: Options) -> Tunnel {
    let (control, controlled) = mpsc::unbounded_channel();
    let (want, wanted) = watch::channel(None);
    let (report, state) = watch::channel(State::down(""));
    tokio::spawn(run_tunnel(
        wanted,
        controlled,
        report,
        options.local,
        options.cloudflared,
    ));
    tokio::spawn(stay_connected(options.auth, want, state));
    Tunnel { control }
}

enum End {
    /// Cloud does not know this machine (any more).
    Unlinked,
    /// The link file changed under the connection.
    Relinked,
}

async fn stay_connected(
    auth: bool,
    want: watch::Sender<Option<Spec>>,
    mut state: watch::Receiver<State>,
) {
    let mut retry = RETRY_MIN;
    loop {
        let stamp = link_stamp();
        let Some(link) = read_link() else {
            let _ = want.send(None);
            tokio::time::sleep(LINK_CHECK).await;
            continue;
        };
        match session(&link, stamp, auth, &want, &mut state).await {
            Ok(End::Relinked) => retry = RETRY_MIN,
            Ok(End::Unlinked) => {
                // Unless it was unlinked from here, a moment ago.
                if read_link().is_some() {
                    tracing::warn!(
                        "{} does not know this machine; link it again with: tilr cloud login",
                        link.url
                    );
                }
                let _ = want.send(None);
                while link_stamp() == stamp {
                    tokio::time::sleep(LINK_CHECK).await;
                }
                retry = RETRY_MIN;
            }
            // The tunnel stays as it is: clients already use it, and Cloud
            // being away is no reason to cut them off.
            Err(err) => {
                tracing::warn!("cloud: {err:#}; trying again in {}s", retry.as_secs());
                tokio::time::sleep(retry).await;
                retry = (retry * 2).min(RETRY_MAX);
            }
        }
    }
}

fn control_url(base: &str) -> Result<String> {
    let rest = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        bail!("{base} is not an http(s) address");
    };
    Ok(format!("{}/v1/machine/connect", rest.trim_end_matches('/')))
}

/// One control connection, from connecting to its end.
async fn session(
    link: &Link,
    stamp: Option<SystemTime>,
    auth: bool,
    want: &watch::Sender<Option<Spec>>,
    state: &mut watch::Receiver<State>,
) -> Result<End> {
    let mut request = control_url(&link.url)?.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {}", link.token).parse()?);
    let connecting = tokio_tungstenite::connect_async(request);
    let mut ws = match tokio::time::timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(tungstenite::Error::Http(response)))
            if response.status() == StatusCode::UNAUTHORIZED =>
        {
            return Ok(End::Unlinked);
        }
        Ok(Err(err)) => return Err(err).with_context(|| format!("connect to {}", link.url)),
        Err(_) => bail!("{} did not answer", link.url),
    };
    let hello = json!({
        "type": "hello",
        "protocol": PROTOCOL,
        "server_id": identity::server_id().to_string(),
        "platform": crate::PLATFORM,
        "version": crate::VERSION,
        "auth": auth,
    });
    ws.send(Message::text(hello.to_string())).await?;

    let mut check = tokio::time::interval(LINK_CHECK);
    let mut checks = 0u32;
    let mut heard = Instant::now();
    loop {
        tokio::select! {
            message = ws.next() => match message.context("connection lost")?? {
                Message::Text(text) if text.as_str() == "pong" => heard = Instant::now(),
                Message::Text(text) => {
                    heard = Instant::now();
                    let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    match message["type"].as_str() {
                        Some("hello_ack") => {
                            tracing::info!(cloud = link.url, machine = link.machine_id, "connected to cloud");
                            let report = state.borrow_and_update().report();
                            ws.send(Message::text(report)).await?;
                        }
                        Some("tunnel_open") if !auth => {
                            let refusal = State::failed("the server runs without authentication");
                            ws.send(Message::text(refusal.report())).await?;
                        }
                        Some("tunnel_open") => match Spec::parse(&message["tunnel"]) {
                            Ok(spec) => {
                                let _ = want.send(Some(spec));
                            }
                            Err(err) => {
                                tracing::warn!("cloud sent a tunnel that cannot be used: {err:#}");
                                let refusal = State::failed(&format!("{err:#}"));
                                ws.send(Message::text(refusal.report())).await?;
                            }
                        },
                        Some("tunnel_close") => {
                            let _ = want.send(None);
                        }
                        Some("error") => {
                            tracing::warn!("cloud: {}", message["message"].as_str().unwrap_or_default());
                        }
                        // From a newer Cloud.
                        _ => {}
                    }
                }
                Message::Close(Some(frame)) if u16::from(frame.code) == CLOSE_REVOKED => {
                    return Ok(End::Unlinked);
                }
                Message::Close(_) => bail!("{} closed the connection", link.url),
                _ => {}
            },
            Ok(()) = state.changed() => {
                let report = state.borrow_and_update().report();
                ws.send(Message::text(report)).await?;
            }
            _ = check.tick() => {
                if link_stamp() != stamp {
                    let _ = want.send(None);
                    let _ = ws.close(None).await;
                    return Ok(End::Relinked);
                }
                ensure!(heard.elapsed() < SILENCE, "{} went silent", link.url);
                checks += 1;
                if checks.is_multiple_of(CHECKS_PER_PING) {
                    ws.send(Message::text("ping")).await?;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The tunnel
// ---------------------------------------------------------------------------

/// A tunnel as Cloud describes it.
#[derive(Clone, PartialEq)]
struct Spec {
    id: String,
    account_tag: String,
    secret: String,
    hostname: String,
}

impl Spec {
    /// Take the description apart. Every part ends up in a file that
    /// `cloudflared` reads, so none may hold anything but what it should.
    fn parse(tunnel: &Value) -> Result<Self> {
        let part = |name: &str, allowed: fn(char) -> bool| {
            let value = tunnel[name].as_str().unwrap_or_default();
            ensure!(
                !value.is_empty() && value.len() <= 253 && value.chars().all(allowed),
                "bad tunnel {name}"
            );
            Ok(value.to_owned())
        };
        Ok(Self {
            id: part("id", |c| c.is_ascii_hexdigit() || c == '-')?,
            account_tag: part("account_tag", |c| c.is_ascii_hexdigit())?,
            secret: part("secret", |c| {
                c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')
            })?,
            hostname: part("hostname", |c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-')
            })?,
        })
    }
}

/// What the server tells Cloud about its tunnel.
#[derive(Clone, PartialEq)]
struct State {
    state: &'static str,
    detail: String,
}

impl State {
    fn new(state: &'static str, detail: &str) -> Self {
        Self {
            state,
            detail: detail.to_owned(),
        }
    }
    fn down(detail: &str) -> Self {
        Self::new("down", detail)
    }
    fn failed(detail: &str) -> Self {
        Self::new("failed", detail)
    }
    fn report(&self) -> String {
        json!({ "type": "tunnel_status", "state": self.state, "detail": self.detail }).to_string()
    }
}

fn tunnel_dir() -> PathBuf {
    clipboard::runtime_dir().join("cloudflared")
}

/// Where the pid of the running `cloudflared` is kept, for a server that
/// starts after this one was killed.
fn pid_file() -> PathBuf {
    tunnel_dir().join("pid")
}

/// Note the pid of the `cloudflared` just started.
fn write_pid(pid: u32) {
    let file = pid_file();
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&file)
        .and_then(|mut out| std::io::Write::write_all(&mut out, format!("{pid}\n").as_bytes()));
    if let Err(err) = written {
        tracing::warn!("tunnel: write {}: {err}", file.display());
    }
}

/// Stop a `cloudflared` that an earlier server on this runtime directory
/// left running (it was killed with SIGKILL, or crashed): the one its pid
/// file names, if that process still runs this directory's configuration.
/// A pid that is gone or runs something else is left alone.
async fn stop_leftover() {
    let file = pid_file();
    let Ok(text) = fs::read_to_string(&file) else {
        return;
    };
    let _ = fs::remove_file(&file);
    let Ok(pid) = text.trim().parse::<libc::pid_t>() else {
        return;
    };
    if pid <= 0 || !runs_config(pid, &tunnel_dir().join("config.yml")) {
        return;
    }
    let gone = || {
        // SAFETY: kill with signal 0 only checks.
        let signalled = unsafe { libc::kill(pid, 0) };
        signalled != 0 || command_line(pid).is_none_or(|line| line.is_empty())
    };
    // SAFETY: kill has no memory-safety preconditions.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let deadline = tokio::time::Instant::now() + TUNNEL_STOP;
    while !gone() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !gone() {
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    tracing::info!("stopped a cloudflared left by an earlier server (pid {pid})");
}

/// Whether process `pid` runs with `config` among its arguments.
fn runs_config(pid: libc::pid_t, config: &Path) -> bool {
    let wanted = config.as_os_str().as_encoded_bytes();
    command_line(pid).is_some_and(|line| line.split(|&byte| byte == 0).any(|arg| arg == wanted))
}

/// The command line of process `pid`, its arguments separated by NULs; empty
/// for a zombie.
#[cfg(target_os = "linux")]
fn command_line(pid: libc::pid_t) -> Option<Vec<u8>> {
    fs::read(format!("/proc/{pid}/cmdline")).ok()
}

/// The command line of process `pid`, its arguments separated by NULs:
/// `KERN_PROCARGS2` (argc, the executable's path, padding, the arguments,
/// the environment).
#[cfg(target_os = "macos")]
fn command_line(pid: libc::pid_t) -> Option<Vec<u8>> {
    let mut argmax: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: sysctl writes at most `size` bytes into `argmax`.
    let got = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut argmax).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if got != 0 || argmax <= 0 {
        return None;
    }
    let mut buf = vec![0u8; argmax as usize];
    let mut size = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: sysctl writes at most `size` bytes into `buf`.
    let got = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if got != 0 {
        return None;
    }
    buf.truncate(size);
    // Past argc: the path, the arguments.
    Some(buf.split_off(std::mem::size_of::<libc::c_int>().min(buf.len())))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn command_line(_pid: libc::pid_t) -> Option<Vec<u8>> {
    None
}

/// Write what `cloudflared` needs to run `spec` and return the command.
/// Where the tunnel leads is said here: this server's port and nothing else.
fn prepare(spec: &Spec, local: SocketAddr, cloudflared: &Path) -> Result<Command> {
    let dir = tunnel_dir();
    clipboard::private_dir(&clipboard::runtime_dir())?;
    clipboard::private_dir(&dir)?;
    let credentials = dir.join("credentials.json");
    let config = dir.join("config.yml");
    let secret = json!({
        "AccountTag": spec.account_tag,
        "TunnelID": spec.id,
        "TunnelSecret": spec.secret,
    });
    for (path, text) in [
        (&credentials, secret.to_string()),
        (
            &config,
            format!(
                "tunnel: {id}\n\
                 credentials-file: {credentials}\n\
                 no-autoupdate: true\n\
                 ingress:\n\
                 \x20 - hostname: {hostname}\n\
                 \x20   service: http://{local}\n\
                 \x20 - service: http_status:404\n",
                id = spec.id,
                credentials = credentials.display(),
                hostname = spec.hostname,
            ),
        ),
    ] {
        fs::write(path, text).with_context(|| format!("write {}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let mut command = Command::new(cloudflared);
    command
        .arg("tunnel")
        .arg("--config")
        .arg(&config)
        .arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // A group of its own: what goes to the server's process group (a
        // terminal's SIGHUP, Ctrl-C) is not for cloudflared.
        .process_group(0);
    #[cfg(target_os = "linux")]
    {
        // SAFETY: getpid has no preconditions.
        let server = unsafe { libc::getpid() };
        // SAFETY: prctl, getppid and _exit are async-signal-safe.
        unsafe {
            command.pre_exec(move || {
                // SIGTERM when the server dies, killed or not, so it signs
                // off at Cloudflare. It is the death of the thread that
                // forked: a worker of the runtime, which lives as long as
                // the server.
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // The server died before that was set.
                if libc::getppid() != server {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
    }
    Ok(command)
}

/// Run `cloudflared` whenever a tunnel is wanted, and say how it is doing.
async fn run_tunnel(
    mut wanted: watch::Receiver<Option<Spec>>,
    mut control: mpsc::UnboundedReceiver<Control>,
    report: watch::Sender<State>,
    local: SocketAddr,
    cloudflared: PathBuf,
) {
    let say = |state: State| {
        report.send_if_modified(|current| {
            let changed = *current != state;
            *current = state;
            changed
        });
    };
    let mut held = false;
    stop_leftover().await;
    loop {
        if held {
            match control.recv().await {
                Some(order) => obey(order, &mut held),
                None => return,
            }
            continue;
        }
        let Some(spec) = wanted.borrow_and_update().clone() else {
            say(State::down(""));
            tokio::select! {
                changed = wanted.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                Some(order) = control.recv() => obey(order, &mut held),
            }
            continue;
        };
        say(State::new("starting", ""));
        let started = prepare(&spec, local, &cloudflared).and_then(|mut command| {
            crate::reload::spawn_unblocked(|| command.spawn())
                .with_context(|| format!("start {}", cloudflared.display()))
        });
        let mut child = match started {
            Ok(child) => child,
            Err(err) => {
                tracing::warn!("tunnel: {err:#}");
                say(State::failed(&format!("{err:#}")));
                tokio::select! {
                    _ = wanted.changed() => {}
                    () = tokio::time::sleep(TUNNEL_RETRY) => {}
                    Some(order) = control.recv() => obey(order, &mut held),
                }
                continue;
            }
        };
        if let Some(pid) = child.id() {
            write_pid(pid);
        }
        tracing::info!(hostname = spec.hostname, "tunnel starting");
        let stderr = child.stderr.take().expect("cloudflared stderr is piped");
        let mut lines = BufReader::new(stderr).lines();
        let ended = loop {
            tokio::select! {
                changed = wanted.changed() => {
                    if changed.is_err() || wanted.borrow().as_ref() != Some(&spec) {
                        break Ended::Unwanted;
                    }
                }
                Some(order) = control.recv() => match order {
                    Control::Hold(done) => break Ended::Held(done),
                    Control::Release => {}
                },
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        if line.contains("Registered tunnel connection") {
                            tracing::info!(hostname = spec.hostname, "tunnel up");
                            say(State::new("up", ""));
                        }
                        tracing::debug!("cloudflared: {line}");
                    }
                    _ => break Ended::Exited(child.wait().await),
                },
            }
        };
        // Once it is gone the pid file goes; a server killed before that
        // leaves it for the next one (`stop_leftover`).
        let forget = || {
            let _ = fs::remove_file(pid_file());
        };
        match ended {
            // Not wanted any more, or another one is.
            Ended::Unwanted => {
                stop(&mut child).await;
                let _ = fs::remove_dir_all(tunnel_dir());
                tracing::info!(hostname = spec.hostname, "tunnel closed");
            }
            // The server is about to replace itself; the tunnel stays as
            // Cloud knows it, and the successor starts its own.
            Ended::Held(done) => {
                stop(&mut child).await;
                forget();
                tracing::info!(hostname = spec.hostname, "tunnel held");
                held = true;
                let _ = done.send(());
            }
            Ended::Exited(exit) => {
                forget();
                let why = match exit {
                    Ok(status) => format!("cloudflared stopped: {status}"),
                    Err(err) => format!("cloudflared stopped: {err}"),
                };
                tracing::warn!("tunnel: {why}");
                say(State::failed(&why));
                tokio::select! {
                    _ = wanted.changed() => {}
                    () = tokio::time::sleep(TUNNEL_RETRY) => {}
                    Some(order) = control.recv() => obey(order, &mut held),
                }
            }
        }
    }
}

/// An order to the tunnel task while no `cloudflared` runs: a hold is
/// answered at once.
fn obey(order: Control, held: &mut bool) {
    match order {
        Control::Hold(done) => {
            *held = true;
            let _ = done.send(());
        }
        Control::Release => *held = false,
    }
}

/// How a running `cloudflared` came to an end.
enum Ended {
    Unwanted,
    Held(oneshot::Sender<()>),
    Exited(std::io::Result<std::process::ExitStatus>),
}

/// Ask `cloudflared` to exit, so it signs off at Cloudflare; kill it if it
/// does not.
async fn stop(child: &mut Child) {
    if let Some(pid) = child.id() {
        // SAFETY: kill has no memory-safety preconditions.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    if tokio::time::timeout(TUNNEL_STOP, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_process_running_this_configuration_is_taken_for_a_leftover() {
        let config = Path::new("/nowhere/tilr-test/cloudflared/config.yml");
        // `; :` keeps the shell from exec'ing sleep, which would drop its
        // arguments.
        let mut ours = std::process::Command::new("sh")
            .args(["-c", "sleep 30; :", "cloudflared", "tunnel", "--config"])
            .arg(config)
            .arg("run")
            .spawn()
            .unwrap();
        let mut other = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = |child: &std::process::Child| child.id() as libc::pid_t;
        // Read once right after `spawn`, the shell's arguments were once not
        // found under load: macOS's /bin/sh is a launcher that execs the
        // real shell again, likely showing no arguments in the middle of
        // that exec. A cloudflared found later counts all the same: wait for
        // them rather than read them once.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !runs_config(pid(&ours), config) {
            assert!(
                Instant::now() < deadline,
                "the shell never showed its arguments"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!runs_config(pid(&other), config));
        assert!(!runs_config(pid(&ours), Path::new("/elsewhere/config.yml")));
        // SAFETY: getpid has no preconditions.
        assert!(!runs_config(unsafe { libc::getpid() }, config));
        for child in [&mut ours, &mut other] {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(!runs_config(pid(&ours), config), "gone");
    }

    fn described(hostname: &str) -> Value {
        json!({
            "id": "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4",
            "account_tag": "0123456789abcdef0123456789abcdef",
            "secret": "c2VjcmV0K3NlY3JldC9zZWNyZXQ9PQ==",
            "hostname": hostname,
        })
    }

    #[test]
    fn a_tunnel_cloud_describes_is_taken() {
        let spec = Spec::parse(&described("abc234.tilr.dev")).unwrap();
        assert_eq!(spec.hostname, "abc234.tilr.dev");
    }

    #[test]
    fn a_tunnel_that_would_rewrite_the_config_is_refused() {
        for hostname in [
            "",
            "a.dev\n  - service: ssh://localhost:22",
            "a b",
            "A.dev:1",
        ] {
            assert!(Spec::parse(&described(hostname)).is_err(), "{hostname:?}");
        }
        assert!(Spec::parse(&json!({})).is_err());
    }

    #[test]
    fn the_default_cloud_is_the_api_host() {
        assert_eq!(DEFAULT_URL, "https://wss.tilr.dev");
        assert_eq!(
            control_url(DEFAULT_URL).unwrap(),
            "wss://wss.tilr.dev/v1/machine/connect"
        );
    }

    #[test]
    fn the_control_address_follows_the_cloud_address() {
        assert_eq!(
            control_url("https://wss.tilr.dev").unwrap(),
            "wss://wss.tilr.dev/v1/machine/connect"
        );
        assert_eq!(
            control_url("http://127.0.0.1:8787/").unwrap(),
            "ws://127.0.0.1:8787/v1/machine/connect"
        );
        assert!(control_url("wss.tilr.dev").is_err());
    }
}
