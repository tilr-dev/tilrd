//! Harness for the end-to-end use cases: starts the real server, talks to it
//! over WebSocket like a client would, and looks at what the shell in a pane
//! sees.

// Each test file uses its own part of the harness.
#![allow(dead_code, unused_imports)]

pub use std::collections::VecDeque;
pub use std::io::{BufRead, BufReader};
pub use std::path::{Path, PathBuf};
pub use std::process::{Child, Command, Stdio};
pub use std::sync::atomic::{AtomicU32, Ordering};
pub use std::sync::{Arc, Mutex};
pub use std::time::Duration;

#[path = "../../../../tests/support/server_owner.rs"]
pub mod server_owner;
pub use server_owner::Owner;

pub use bytes::Bytes;
pub use futures_util::{SinkExt, StreamExt};
pub use tilr_proto::{
    ClientMessage, ClipboardAck, ClipboardPut, ClipboardWrite, ClosePane, CreatePane, Frame,
    FsRead, Hello, HelloAck, HostEvent, KeyInput, ListPanes, PROTOCOL_VERSION, PaneInfo, Ping,
    RawInput, Resize, Scroll, ServerMessage, Subscribe, Unsubscribe, client_message::Msg,
    server_message,
};
pub use tokio::net::TcpStream;
pub use tokio_tungstenite::tungstenite::Message;
pub use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// A pane's id, in the form that is handy to pass around; it goes over the
/// wire as text.
pub type PaneId = uuid::Uuid;

pub const TIMEOUT: Duration = Duration::from_secs(20);
pub const CHUNK: usize = 256 * 1024;

/// Event-driven control of `pause` failpoints. Bind before triggering the
/// operation; keep each returned hit until it is time for that hit to proceed.
#[cfg(feature = "failpoints")]
pub struct PauseController(tokio::net::UnixListener);

#[cfg(feature = "failpoints")]
impl PauseController {
    pub fn bind(dir: &Path) -> Self {
        Self(tokio::net::UnixListener::bind(dir.join("pause.sock")).unwrap())
    }

    pub async fn wait(&self, name: &str) -> PausedPoint {
        use tokio::io::AsyncBufReadExt;
        tokio::time::timeout(TIMEOUT, async {
            let (socket, _) = self.0.accept().await.unwrap();
            let mut socket = tokio::io::BufReader::new(socket);
            let mut reached = String::new();
            socket.read_line(&mut reached).await.unwrap();
            assert_eq!(reached, format!("{name}\n"), "unexpected paused point");
            PausedPoint(socket.into_inner())
        })
        .await
        .unwrap_or_else(|_| panic!("did not reach pause {name}"))
    }
}

/// One paused invocation. Dropping it without release fails the point.
#[cfg(feature = "failpoints")]
pub struct PausedPoint(tokio::net::UnixStream);

#[cfg(feature = "failpoints")]
impl PausedPoint {
    pub async fn release(mut self) {
        use tokio::io::AsyncWriteExt;
        tokio::time::timeout(TIMEOUT, self.0.write_all(b"r"))
            .await
            .expect("pause release timed out")
            .expect("pause release failed");
    }
}

/// A running server with a scratch directory of its own.
pub struct Server {
    pub owner: Owner,
    pub child: Child,
    pub addr: String,
    /// Scratch directory: the server's runtime directory lives in it, and
    /// tests put their own files there.
    pub dir: PathBuf,
    pub log: Arc<Mutex<Vec<String>>>,
    pub log_changed: Arc<tokio::sync::Notify>,
}

/// Copy diagnostics before panicking: no log guard may survive into unwinding.
/// Cleanup must also work if another user of the shared log poisoned its lock.
fn log_text(log: &Mutex<Vec<String>>) -> String {
    log.lock().unwrap_or_else(|err| err.into_inner()).join("\n")
}

/// Spawn `command`, whose executable may be a copy made a moment ago. On
/// Linux such a copy is "busy" (ETXTBSY) while a child forked by another
/// test thread still holds the copy's write descriptor (it is closed on
/// exec, not on fork); that passes as soon as the child execs.
pub fn spawn_copied(command: &mut Command) -> Child {
    let mut tries = 0;
    loop {
        match command.spawn() {
            Ok(child) => return child,
            Err(err) if err.raw_os_error() == Some(libc::ETXTBSY) && tries < 100 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("spawn {:?}: {err}", command.get_program()),
        }
    }
}

impl Server {
    pub fn start(args: &[&str]) -> Self {
        Self::start_exe(Path::new(env!("CARGO_BIN_EXE_tilrd")), args)
    }

    pub fn scratch_dir() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tilr-it-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn start_exe(exe: &Path, args: &[&str]) -> Self {
        Self::start_in(Self::scratch_dir(), exe, args)
    }

    pub fn start_in(dir: PathBuf, exe: &Path, args: &[&str]) -> Self {
        Self::start_failing(dir, exe, args, "")
    }

    /// Start a server in which things go wrong where `failpoints` says, see
    /// `src/fail.rs`. It hands them on to what it reloads into.
    pub fn start_failing(dir: PathBuf, exe: &Path, args: &[&str], failpoints: &str) -> Self {
        Self::start_full(dir, exe, args, failpoints, &[])
    }

    /// Start a server whose environment also has `env`, e.g. what a service
    /// manager sets.
    pub fn start_with_env(dir: PathBuf, args: &[&str], env: &[(&str, &str)]) -> Self {
        Self::start_full(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), args, "", env)
    }

    pub fn start_full(
        dir: PathBuf,
        exe: &Path,
        args: &[&str],
        failpoints: &str,
        env: &[(&str, &str)],
    ) -> Self {
        let mut command = Command::new(exe);
        command.args(["--listen", "127.0.0.1:0"]).args(args);
        Self::environment(&mut command, &dir, failpoints);
        command.envs(env.iter().copied());
        command.stdin(Stdio::null()).stderr(Stdio::piped());
        let mut server = Self::spawn_owned(command, dir);
        let mut lines = BufReader::new(server.child.stderr.take().unwrap()).lines();
        let log = Arc::clone(&server.log);
        let log_changed = Arc::clone(&server.log_changed);
        let mut addr = None;
        for line in lines.by_ref() {
            let line = line.unwrap();
            let found = line
                .split("listening on ")
                .nth(1)
                .map(|addr| addr.trim().to_owned());
            log.lock().unwrap().push(line);
            if found.is_some() {
                addr = found;
                break;
            }
        }
        let addr = addr.unwrap_or_else(|| panic!("server did not start: {}", log_text(&log)));
        // Keep draining the log so the server never blocks on it.
        let sink = Arc::clone(&log);
        let changed = Arc::clone(&log_changed);
        std::thread::spawn(move || {
            for line in lines.map_while(Result::ok) {
                sink.lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push(line);
                changed.notify_waiters();
            }
        });
        server.addr = addr;
        server
    }

    /// Own the process before reading any startup output that may panic.
    pub fn spawn_owned(mut command: Command, dir: PathBuf) -> Self {
        let owner = Owner::new(&dir);
        owner.configure(&mut command);
        let child = spawn_copied(&mut command);
        Self {
            owner,
            child,
            addr: String::new(),
            dir,
            log: Arc::default(),
            log_changed: Arc::default(),
        }
    }

    /// The environment a server under test runs in: its directories under
    /// `dir`, failures where `failpoints` says, nothing of the machine's.
    pub fn environment(command: &mut Command, dir: &Path, failpoints: &str) {
        command
            .env("TILR_FAILPOINTS", failpoints)
            .env("TILR_RUNTIME_DIR", dir.join("runtime"))
            .env("TILR_STATE_DIR", dir.join("state"))
            .env("TILR_CONFIG_DIR", dir.join("config"))
            .env("TILR_FAILPOINTS_FILE", dir.join("failpoints"))
            .env("TILR_FAILPOINTS_SOCKET", dir.join("pause.sock"))
            .env("HOME", dir)
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .env("PS1", "$ ")
            // Each test opts into its own clock through `start_with_env`.
            .env_remove("TILR_TEST_TIME_FILE")
            // No real clipboard for the shims to fall back to, and no shell
            // startup file.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("ENV")
            // Nor an SSH agent of the machine running the tests, nor the
            // clipboard of a Tilr pane the tests may run in.
            .env_remove("SSH_AUTH_SOCK")
            .env_remove("TILR_CLIPBOARD_DIR")
            // Nor the Unix socket of the server whose pane the tests may
            // run in: each server's goes in its own runtime directory.
            .env_remove("TILR_SOCKET")
            // Nor the log filter of the shell running the tests: what a
            // server under test logs is the default.
            .env_remove("RUST_LOG");
    }

    /// Whether the server has logged a line with `needle` so far, as far as
    /// the thread reading its log has got. For a line an event makes the
    /// server write, wait with [`Server::wait_logged`] instead: the line
    /// reaches this log some time after the event.
    pub fn logged(&self, needle: &str) -> bool {
        self.log
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .any(|line| line.contains(needle))
    }

    /// Wait up to [`TIMEOUT`] for the server to log a line with `needle`.
    pub async fn wait_logged(&self, needle: &str) {
        self.wait_logged_within(needle, TIMEOUT).await;
    }

    /// Wait up to `within` for the server to log a line with `needle`.
    pub async fn wait_logged_within(&self, needle: &str, within: Duration) {
        let deadline = tokio::time::Instant::now() + within;
        while !self.logged(needle) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "not logged within {within:?}: {needle}\n{}",
                log_text(&self.log)
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// [`Server::wait_logged`] outside of an async context.
    pub fn wait_logged_blocking(&self, needle: &str) {
        self.wait_logged_blocking_within(needle, TIMEOUT);
    }

    /// The blocking log wait with a caller-provided deadline.
    pub fn wait_logged_blocking_within(&self, needle: &str, within: Duration) {
        let deadline = std::time::Instant::now() + within;
        while !self.logged(needle) {
            assert!(
                std::time::Instant::now() < deadline,
                "not logged within {within:?}: {needle}\n{}",
                log_text(&self.log)
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The access token the server made for itself; empty if it runs
    /// without one.
    pub fn token(&self) -> String {
        std::fs::read_to_string(self.dir.join("state/token"))
            .map(|token| token.trim().to_owned())
            .unwrap_or_default()
    }

    pub fn signal(&self, signal: &str) {
        let status = Command::new("kill")
            .args([signal, &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.owner.close();
        let _ = self.child.wait();
        // A server killed outright leaves its host socket behind. Where the
        // scratch directory's path is too long for a socket (macOS), that
        // is in /tmp.
        if let Ok(id) = std::fs::read_to_string(self.dir.join("state/server-id")) {
            // SAFETY: geteuid has no preconditions.
            let uid = unsafe { libc::geteuid() };
            let sock = format!("/tmp/tilr-{uid}/{}.sock", id.trim());
            let _ = std::fs::remove_file(sock);
            // That of a second server of the same id, named by its pid.
            let sock = format!("/tmp/tilr-{uid}/{}-{}.sock", id.trim(), self.child.id());
            let _ = std::fs::remove_file(sock);
        }
        if std::thread::panicking() {
            eprintln!("--- server log ---\n{}", log_text(&self.log));
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a client sees of one pane: the rows of its screen.
#[derive(Default)]
pub struct Screen {
    pub rows: Vec<String>,
}

impl Screen {
    pub fn apply(&mut self, frame: &Frame) {
        if frame.full {
            self.rows = vec![String::new(); frame.rows as usize];
        }
        for row in &frame.changed {
            let text = row.runs.iter().map(|run| run.text.as_str()).collect();
            if let Some(slot) = self.rows.get_mut(row.y as usize) {
                *slot = text;
            }
        }
    }

    pub fn text(&self) -> String {
        self.rows.join("\n")
    }
}

/// An explicit release for socket reads. Dropping it without resuming fails
/// the next read; it never silently resumes a paused client.
pub struct ReadPause(tokio::sync::oneshot::Sender<()>);

impl ReadPause {
    pub fn resume(self) {
        self.0.send(()).expect("paused client disappeared");
    }
}

pub struct Client {
    pub ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub screen: Screen,
    /// The last frame received, without its rows.
    pub frame: Option<Frame>,
    /// Clipboard acks seen while waiting for something else.
    pub acks: VecDeque<ClipboardAck>,
    /// Host events seen while waiting for something else.
    pub host_events: VecDeque<HostEvent>,
    /// `ClipboardWrite` chunks seen while waiting for something else.
    pub copies: VecDeque<ClipboardWrite>,
    /// Pane activity seen while waiting for another message.
    pub activities: VecDeque<tilr_proto::PaneActivity>,
    /// The server's access token, as `tilrd token` prints it.
    pub token: String,
    /// What `hello` says as `Hello.client_id`; empty: none.
    pub client_id: String,
    reads_paused: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl Client {
    pub async fn connect(server: &Server) -> Self {
        Self::try_connect(server).await.expect("connect")
    }

    pub async fn try_connect(server: &Server) -> Option<Self> {
        let mut client = Self::try_connect_silent(server).await?;
        client.hello().await;
        Some(client)
    }

    /// Connect without saying hello.
    pub async fn connect_silent(server: &Server) -> Self {
        Self::try_connect_silent(server).await.expect("connect")
    }

    async fn try_connect_silent(server: &Server) -> Option<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(&server.addr).await.ok()?;
        Some(Self::from_socket(server, ws))
    }

    pub fn from_socket(server: &Server, ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        Self {
            ws,
            screen: Screen::default(),
            frame: None,
            acks: VecDeque::new(),
            host_events: VecDeque::new(),
            copies: VecDeque::new(),
            activities: VecDeque::new(),
            token: server.token(),
            client_id: String::new(),
            reads_paused: None,
        }
    }

    /// Acknowledged on return: this client has no background reader, and
    /// every subsequent `next` waits for the returned handle's `resume`.
    /// Sending messages remains available while reads are paused.
    pub fn pause_reads(&mut self) -> ReadPause {
        assert!(self.reads_paused.is_none(), "reads already paused");
        let (release, paused) = tokio::sync::oneshot::channel();
        self.reads_paused = Some(paused);
        ReadPause(release)
    }

    pub async fn send(&mut self, msg: Msg) {
        let packet = tilr_proto::encode(&ClientMessage::from(msg));
        self.ws.send(Message::Binary(packet)).await.expect("send");
    }

    /// The next message, or `None` once the server closed the connection.
    /// Fails if none comes within [`TIMEOUT`]: the server's pings, which come
    /// all the time, do not count.
    pub async fn next(&mut self) -> Option<server_message::Msg> {
        if let Some(paused) = &mut self.reads_paused {
            paused.await.expect("read pause dropped without resume");
            self.reads_paused = None;
        }
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let message = tokio::time::timeout_at(deadline, self.ws.next())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "timed out waiting for the server; screen:\n{}",
                        self.screen.text()
                    )
                });
            match message {
                Some(Ok(Message::Binary(packet))) => {
                    let msg = tilr_proto::decode::<ServerMessage>(&packet)
                        .expect("decode")
                        .msg;
                    match &msg {
                        Some(server_message::Msg::Frame(frame)) => {
                            self.screen.apply(frame);
                            self.frame = Some(Frame {
                                changed: Vec::new(),
                                styles: Vec::new(),
                                ..frame.clone()
                            });
                        }
                        Some(server_message::Msg::ClipboardAck(ack)) => {
                            self.acks.push_back(ack.clone());
                        }
                        Some(server_message::Msg::HostEvent(event)) => {
                            self.host_events.push_back(event.clone());
                        }
                        Some(server_message::Msg::ClipboardWrite(chunk)) => {
                            self.copies.push_back(chunk.clone());
                        }
                        Some(server_message::Msg::PaneActivity(activity)) => {
                            self.activities.push_back(activity.clone());
                        }
                        _ => {}
                    }
                    if msg.is_some() {
                        return msg;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
                Some(Ok(_)) => {}
            }
        }
    }

    /// Read messages until `pick` takes one.
    pub async fn wait_for<T>(
        &mut self,
        mut pick: impl FnMut(&server_message::Msg) -> Option<T>,
    ) -> T {
        loop {
            let msg = self.next().await.expect("connection closed");
            if let Some(picked) = pick(&msg) {
                return picked;
            }
        }
    }

    pub async fn hello(&mut self) -> HelloAck {
        self.hello_as(PROTOCOL_VERSION).await;
        self.wait_for(|msg| match msg {
            server_message::Msg::HelloAck(ack) => Some(ack.clone()),
            _ => None,
        })
        .await
    }

    /// Say hello as a client built for `protocol_version`, without waiting
    /// for the answer.
    pub async fn hello_as(&mut self, protocol_version: u32) {
        self.hello_with(protocol_version, &["clipboard", "host-events"])
            .await;
    }

    /// Say hello announcing `features`, without waiting for the answer.
    pub async fn hello_with(&mut self, protocol_version: u32, features: &[&str]) {
        self.send(Msg::Hello(Hello {
            protocol_version,
            client_name: "test".into(),
            features: features.iter().map(|f| (*f).to_owned()).collect(),
            token: self.token.clone(),
            client_id: self.client_id.clone(),
        }))
        .await;
    }

    pub async fn list_panes(&mut self) -> Vec<PaneInfo> {
        self.send(Msg::ListPanes(ListPanes { request_id: 77 }))
            .await;
        self.wait_for(|msg| match msg {
            server_message::Msg::PaneList(list) if list.request_id == 77 => {
                Some(list.panes.clone())
            }
            _ => None,
        })
        .await
    }

    /// Start a shell in a new pane and watch it.
    pub async fn open_pane(&mut self) -> PaneId {
        self.open_pane_with(100, 30, "").await
    }

    pub async fn open_pane_with(&mut self, cols: u32, rows: u32, cwd: &str) -> PaneId {
        self.send(Msg::CreatePane(CreatePane {
            cols,
            rows,
            command: vec!["/bin/sh".into()],
            cwd: cwd.into(),
            request_id: 5,
        }))
        .await;
        let pane_id = self
            .wait_for(|msg| match msg {
                server_message::Msg::PaneCreated(created) if created.request_id == 5 => {
                    Some(created.pane.as_ref().unwrap().pane_id.parse().unwrap())
                }
                server_message::Msg::Error(error) => panic!("create pane: {}", error.message),
                _ => None,
            })
            .await;
        self.subscribe(pane_id).await;
        pane_id
    }

    pub async fn subscribe(&mut self, pane_id: PaneId) {
        self.screen = Screen::default();
        self.frame = None;
        self.send(Msg::Subscribe(Subscribe {
            pane_id: pane_id.to_string(),
        }))
        .await;
    }

    /// Type a line into the pane's shell.
    pub async fn type_line(&mut self, pane_id: PaneId, line: &str) {
        self.send(Msg::Raw(RawInput {
            pane_id: pane_id.to_string(),
            data: Bytes::from(format!("{line}\n")),
        }))
        .await;
    }

    /// Wait until the pane's screen shows `text`.
    pub async fn wait_screen(&mut self, text: &str) {
        while !self.screen.text().contains(text) {
            let msg = self.next().await;
            assert!(
                msg.is_some(),
                "connection closed; screen:\n{}",
                self.screen.text()
            );
        }
    }

    /// Run `command` in the pane's shell and return its exit status. The
    /// marker is typed split in two, so only the shell's output matches.
    pub async fn run(&mut self, pane_id: PaneId, command: &str) -> i32 {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        self.type_line(pane_id, &format!("{command}; echo \"<st\"\"atus{n}:$?>\""))
            .await;
        let marker = format!("<status{n}:");
        self.wait_screen(&marker).await;
        loop {
            let text = self.screen.text();
            let rest = text.split(&marker).nth(1).unwrap();
            if let Some(end) = rest.find('>') {
                return rest[..end].parse().expect("exit status");
            }
            self.next().await.expect("connection closed");
        }
    }

    /// Upload `data` to the server's clipboard in chunks, without waiting
    /// for the outcome.
    pub async fn put_clipboard(&mut self, transfer_id: u64, mime: &str, data: &[u8]) {
        let chunks: Vec<&[u8]> = data.chunks(CHUNK).collect();
        for (index, chunk) in chunks.iter().enumerate() {
            let first = index == 0;
            self.send(Msg::ClipboardPut(ClipboardPut {
                transfer_id,
                mime: if first { mime.into() } else { String::new() },
                total_size: if first { data.len() as u64 } else { 0 },
                data: Bytes::copy_from_slice(chunk),
                last: index + 1 == chunks.len(),
            }))
            .await;
        }
    }

    /// Say hello as a client that takes the copies made in its panes
    /// (`"clipboard-write"`).
    pub async fn hello_copying(&mut self) -> HelloAck {
        self.hello_with(
            PROTOCOL_VERSION,
            &["clipboard", "host-events", "clipboard-write"],
        )
        .await;
        self.wait_for(|msg| match msg {
            server_message::Msg::HelloAck(ack) => Some(ack.clone()),
            _ => None,
        })
        .await
    }

    /// The oldest copy not yet asked about that came whole, with the
    /// transfers before it that ended without their last chunk.
    pub async fn copied(&mut self) -> Copied {
        let mut copied: Option<Copied> = None;
        let mut abandoned = Vec::new();
        loop {
            while let Some(chunk) = self.copies.pop_front() {
                match &mut copied {
                    Some(copy) if copy.transfer_id == chunk.transfer_id => {
                        copy.data.extend_from_slice(&chunk.data);
                        copy.chunks += 1;
                    }
                    _ => {
                        if let Some(older) = copied.take() {
                            abandoned.push(older.transfer_id);
                        }
                        copied = Some(Copied {
                            transfer_id: chunk.transfer_id,
                            pane_id: chunk.pane_id.clone(),
                            mime: chunk.mime.clone(),
                            total_size: chunk.total_size,
                            data: chunk.data.to_vec(),
                            chunks: 1,
                            abandoned: Vec::new(),
                        });
                    }
                }
                if chunk.last {
                    let mut copy = copied.take().unwrap();
                    copy.abandoned = abandoned;
                    return copy;
                }
            }
            self.next().await.expect("connection closed");
        }
    }

    /// Read a small file through the files protocol: its answer comes on
    /// the bulk lane, behind whatever the server queued there before.
    pub async fn sync_bulk(&mut self, server: &Server) {
        self.read_through_bulk(server, 4).await;
    }

    /// Read a file of `len` bytes through the files protocol, to its last
    /// chunk: the bulk lane gives room to its chunks and to those of other
    /// senders waiting for it in turn.
    pub async fn read_through_bulk(&mut self, server: &Server, len: usize) {
        let path = server.dir.join("bulk-sync");
        std::fs::write(&path, vec![b's'; len]).unwrap();
        self.send(Msg::FsRead(FsRead {
            request_id: 4242,
            path: path.to_str().unwrap().into(),
            ..Default::default()
        }))
        .await;
        self.wait_for(|msg| match msg {
            server_message::Msg::FsData(data) if data.request_id == 4242 && data.last => Some(()),
            _ => None,
        })
        .await;
    }

    /// The outcome of the oldest upload not yet asked about.
    pub async fn clipboard_ack(&mut self) -> ClipboardAck {
        loop {
            if let Some(ack) = self.acks.pop_front() {
                return ack;
            }
            self.next().await.expect("connection closed");
        }
    }
}

/// Bytes that look like nothing in particular, newlines and zeros included.
/// A copy made in a pane, as a client got it.
#[derive(Debug)]
pub struct Copied {
    pub transfer_id: u64,
    pub pane_id: String,
    pub mime: String,
    pub total_size: u64,
    pub data: Vec<u8>,
    pub chunks: usize,
    /// Transfers that came before it and ended without their last chunk.
    pub abandoned: Vec<u64>,
}

pub fn image(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
    while data.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        data.push(state as u8);
    }
    data
}

/// The shell commands Claude Code 2.1.289 runs on Linux to paste an image:
/// first to see whether the clipboard holds one, then to save it to a file.
pub const CLAUDE_CHECK_IMAGE: &str = r#"xclip -selection clipboard -t TARGETS -o 2>/dev/null | grep -E "image/(png|jpeg|jpg|gif|webp|bmp)" || wl-paste -l 2>/dev/null | grep -E "image/(png|jpeg|jpg|gif|webp|bmp)""#;

pub fn claude_save_image(file: &Path) -> String {
    let file = file.display();
    format!(
        "xclip -selection clipboard -t image/png -o > '{file}' 2>/dev/null || wl-paste --type image/png > '{file}' 2>/dev/null || xclip -selection clipboard -t image/bmp -o > '{file}' 2>/dev/null || wl-paste --type image/bmp > '{file}'"
    )
}

/// Write `script` to a file in the server's scratch directory and return the
/// command that runs it. Keeps long commands off the pane's command line.
pub fn script(server: &Server, name: &str, script: &str) -> String {
    let path = server.dir.join(name);
    std::fs::write(&path, script).unwrap();
    format!("sh '{}'", path.display())
}

/// A FIFO kept open at both ends so sending a short release never waits for
/// a reader. The command under test still blocks until we write a line.
pub fn activity_fifo(server: &Server) -> (PathBuf, std::fs::File) {
    let fifo = server.dir.join("activity-release");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let release = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&fifo)
        .unwrap();
    (fifo, release)
}

pub fn release_activity_fifo(release: &mut std::fs::File) {
    use std::io::Write;
    release.write_all(b"continue\n").unwrap();
}

/// Start a second server in the scratch directory of `first`, a running
/// one: one user's two servers, with one state directory (so one server id
/// and token) and Unix sockets of their own.
///
/// The host socket goes to whichever server binds the name first: the
/// first server's `listening on`, at which [`Server::start`] returns, comes
/// before it binds its host socket, so the second waits for that. It
/// returns once the second has bound its own.
pub fn start_second(first: &Server) -> Server {
    first.wait_logged_blocking("host events on");
    let dir = &first.dir;
    let socket = dir.join("second.sock");
    let second = Server::start_in(
        dir.to_owned(),
        Path::new(env!("CARGO_BIN_EXE_tilrd")),
        &["--socket", socket.to_str().unwrap()],
    );
    second.wait_logged_blocking("host events on");
    second
}

/// What `$TILR_SOCK` is in the pane.
pub async fn pane_sock(server: &Server, client: &mut Client, pane_id: PaneId) -> PathBuf {
    let file = format!("sock-{pane_id}");
    let command = format!("printf %s \"$TILR_SOCK\" > ~/{file}");
    assert_eq!(client.run(pane_id, &command).await, 0);
    PathBuf::from(std::fs::read_to_string(server.dir.join(file)).unwrap())
}

/// Wait until the server has exited after SIGTERM, without removing its
/// directory (which a second server may share).
pub async fn stop(server: &mut Server) {
    server.signal("-TERM");
    eventually("the server exits", || !server.is_running()).await;
}

/// What a service manager hands a `Type=notify` unit: `NOTIFY_SOCKET` (a
/// datagram socket in `dir`, returned to read `READY=1` from), socket
/// activation and the watchdog.
pub fn service_manager_env(
    dir: &Path,
) -> (std::os::unix::net::UnixDatagram, Vec<(String, String)>) {
    let path = dir.join("notify.sock");
    let socket = std::os::unix::net::UnixDatagram::bind(&path).unwrap();
    let env = [
        ("NOTIFY_SOCKET", path.to_str().unwrap()),
        ("LISTEN_FDS", "1"),
        ("LISTEN_FDNAMES", "tilrd"),
        ("LISTEN_PID", "1"),
        ("WATCHDOG_USEC", "30000000"),
        ("WATCHDOG_PID", "1"),
    ];
    let env = env
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    (socket, env)
}

/// Wait for the server to drop the connection, then connect again.
pub async fn reconnect(server: &Server, mut client: Client) -> Client {
    while client.next().await.is_some() {}
    for _ in 0..100 {
        if let Some(client) = Client::try_connect(server).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("server did not come back");
}

impl Screen {
    /// Whether some row holds exactly `text`.
    pub fn has_row(&self, text: &str) -> bool {
        self.rows.iter().any(|row| row.trim_end() == text)
    }
}

impl Client {
    /// Connect the way a page served from `origin` would.
    pub async fn connect_from(server: &Server, origin: &str) -> Result<Self, String> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = server.addr.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Ok((ws, _)) => Ok(Self {
                ws,
                screen: Screen::default(),
                frame: None,
                acks: VecDeque::new(),
                host_events: VecDeque::new(),
                copies: VecDeque::new(),
                activities: VecDeque::new(),
                token: server.token(),
                client_id: String::new(),
                reads_paused: None,
            }),
            Err(err) => Err(err.to_string()),
        }
    }

    /// Wait until some row of the pane's screen is exactly `text`.
    pub async fn wait_row(&mut self, text: &str) {
        while !self.screen.has_row(text) {
            let msg = self.next().await;
            assert!(
                msg.is_some(),
                "connection closed waiting for row {text:?}; screen:\n{}",
                self.screen.text()
            );
        }
    }

    /// Wait until no row of the pane's screen is `text`.
    pub async fn wait_no_row(&mut self, text: &str) {
        while self.screen.has_row(text) {
            self.next().await.expect("connection closed");
        }
    }

    /// Print `line-1` to `line-<count>` in the pane, one per row.
    pub async fn print_lines(&mut self, pane_id: PaneId, count: u32) {
        let command =
            format!("i=1; while [ $i -le {count} ]; do echo \"line-$i\"; i=$((i+1)); done");
        assert_eq!(self.run(pane_id, &command).await, 0);
    }

    /// Move the pane's viewport: negative `delta` goes back into history.
    pub async fn scroll(&mut self, pane_id: PaneId, delta: i32) {
        self.send(Msg::Scroll(Scroll {
            pane_id: pane_id.to_string(),
            delta,
            to_bottom: false,
        }))
        .await;
    }

    pub async fn scroll_to_bottom(&mut self, pane_id: PaneId) {
        self.send(Msg::Scroll(Scroll {
            pane_id: pane_id.to_string(),
            delta: 0,
            to_bottom: true,
        }))
        .await;
    }

    /// Press the key of a plain character.
    pub async fn press(&mut self, pane_id: PaneId, key: char) {
        self.send(Msg::Key(KeyInput {
            pane_id: pane_id.to_string(),
            codepoint: u32::from(key),
            text: key.to_string(),
            ..Default::default()
        }))
        .await;
    }

    /// Ask the server whether it is still there, without waiting for the
    /// answer.
    pub async fn ping(&mut self, nonce: u64) {
        self.send(Msg::Ping(Ping { nonce })).await;
    }

    /// Whether the answer to the ping with `nonce` comes within `limit`.
    pub async fn pong_within(&mut self, nonce: u64, limit: Duration) -> bool {
        let pong = self.wait_for(|msg| match msg {
            server_message::Msg::Pong(pong) if pong.nonce == nonce => Some(()),
            _ => None,
        });
        tokio::time::timeout(limit, pong).await.is_ok()
    }

    /// The error the server sends next.
    pub async fn error(&mut self) -> String {
        self.wait_for(|msg| match msg {
            server_message::Msg::Error(error) => Some(error.message.clone()),
            _ => None,
        })
        .await
    }

    /// The pane list the server pushes next on its own, after a change.
    pub async fn pushed_pane_list(&mut self) -> Vec<PaneInfo> {
        self.wait_for(|msg| match msg {
            server_message::Msg::PaneList(list) if list.request_id == 0 => Some(list.panes.clone()),
            _ => None,
        })
        .await
    }

    pub async fn pane_exit_code(&mut self, pane_id: PaneId) -> i32 {
        self.wait_for(|msg| match msg {
            server_message::Msg::PaneExited(exited) if exited.pane_id == pane_id.to_string() => {
                Some(exited.exit_code)
            }
            _ => None,
        })
        .await
    }
}

/// Whether a process with this pid is running. A zombie, which has exited
/// and only waits to be reaped, does not count.
pub fn process_alive(pid: u32) -> bool {
    let state = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default();
    !state.is_empty() && !state.starts_with('Z')
}

/// The command line of a process, as `ps` shows it.
pub fn process_args(pid: u32) -> String {
    let output = Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A running pane with a title-refresh pause hook ready for inspection.
#[cfg(feature = "failpoints")]
pub struct TitleTest {
    pub server: Server,
    pub controller: PauseController,
    pub client: Client,
    pub pane: PaneId,
    pub shell: u32,
    pub pid: u32,
    pub hook: String,
    pub points: PathBuf,
}

#[cfg(feature = "failpoints")]
impl TitleTest {
    pub async fn start() -> Self {
        let server = Server::start(&[]);
        let controller = PauseController::bind(&server.dir);
        let mut client = Client::connect(&server).await;
        let pane = client.open_pane_with(100, 30, "").await;
        let shell = shell_pid(&server, &mut client, pane).await;
        let pid = client.list_panes().await[0].pid;
        let hook = format!("pane-title-polled:{pane}");
        let points = server.dir.join("failpoints");
        std::fs::write(&points, format!("{hook}=pause")).unwrap();
        Self {
            server,
            controller,
            client,
            pane,
            shell,
            pid,
            hook,
            points,
        }
    }
}

/// Read ps after a concrete title refresh, keeping the pane paused for inspection.
#[cfg(feature = "failpoints")]
pub async fn wait_process_title(
    controller: &PauseController,
    hook: &str,
    pid: u32,
    suffix: &str,
) -> PausedPoint {
    let mut last = String::new();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let point = controller.wait(hook).await;
            last = process_args(pid);
            if last.ends_with(suffix) {
                return point;
            }
            point.release().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("process {pid} did not show {suffix:?}; last title: {last:?}"))
}

/// The pid of the shell in the pane, which the shell itself writes to a file.
pub async fn shell_pid(server: &Server, client: &mut Client, pane_id: PaneId) -> u32 {
    let file = server.dir.join(format!("shell-pid-{pane_id}"));
    let command = format!("echo $$ > '{}'", file.display());
    assert_eq!(client.run(pane_id, &command).await, 0);
    std::fs::read_to_string(&file)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// Wait up to five seconds for `condition` to hold.
pub async fn eventually(what: &str, condition: impl FnMut() -> bool) {
    eventually_within(what, Duration::from_secs(5), condition).await;
}

/// Look up to `within` for `condition` to hold, every 100 ms.
pub async fn eventually_within(what: &str, within: Duration, mut condition: impl FnMut() -> bool) {
    for _ in 0..within.as_millis().div_ceil(100) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("did not happen within {within:?}: {what}");
}

/// Ask the server to reload and connect again once it has.
pub async fn reload(server: &Server, client: Client) -> Client {
    server.signal("-USR1");
    reconnect(server, client).await
}
