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

pub use bytes::Bytes;
pub use futures_util::{SinkExt, StreamExt};
pub use tilr_proto::{
    ClientMessage, ClipboardAck, ClipboardPut, ClosePane, CreatePane, Frame, Hello, HelloAck,
    KeyInput, ListPanes, PROTOCOL_VERSION, PaneInfo, Ping, RawInput, Resize, Scroll, ServerMessage,
    Subscribe, Unsubscribe, client_message::Msg, server_message,
};
pub use tokio::net::TcpStream;
pub use tokio_tungstenite::tungstenite::Message;
pub use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// A pane's id, in the form that is handy to pass around; it goes over the
/// wire as text.
pub type PaneId = uuid::Uuid;

pub const TIMEOUT: Duration = Duration::from_secs(20);
pub const CHUNK: usize = 256 * 1024;

/// A running server with a scratch directory of its own.
pub struct Server {
    pub child: Child,
    pub addr: String,
    /// Scratch directory: the server's runtime directory lives in it, and
    /// tests put their own files there.
    pub dir: PathBuf,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    pub fn start(args: &[&str]) -> Self {
        Self::start_exe(Path::new(env!("CARGO_BIN_EXE_tilr-server")), args)
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
        let mut command = Command::new(exe);
        command
            .env("TILR_FAILPOINTS", failpoints)
            .args(["--listen", "127.0.0.1:0"])
            .args(args)
            .env("TILR_RUNTIME_DIR", dir.join("runtime"))
            .env("TILR_STATE_DIR", dir.join("state"))
            .env("TILR_FAILPOINTS_FILE", dir.join("failpoints"))
            .env("HOME", &dir)
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .env("PS1", "$ ")
            // No real clipboard for the shims to fall back to, and no shell
            // startup file.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("ENV")
            .stdin(Stdio::null())
            .stderr(Stdio::piped());
        // A copy of the executable made a moment ago can be "busy" on Linux
        // while a child forked by another test still holds it open for
        // writing; that passes as soon as the child execs.
        let mut tries = 0;
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(err) if err.raw_os_error() == Some(libc::ETXTBSY) && tries < 100 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(err) => panic!("start server: {err}"),
            }
        };

        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let log = Arc::new(Mutex::new(Vec::new()));
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
        let addr =
            addr.unwrap_or_else(|| panic!("server did not start: {:?}", log.lock().unwrap()));
        // Keep draining the log so the server never blocks on it.
        let sink = Arc::clone(&log);
        std::thread::spawn(move || {
            for line in lines.map_while(Result::ok) {
                sink.lock().unwrap().push(line);
            }
        });
        Self {
            child,
            addr,
            dir,
            log,
        }
    }

    pub fn logged(&self, needle: &str) -> bool {
        self.log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(needle))
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
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            eprintln!(
                "--- server log ---\n{}",
                self.log.lock().unwrap().join("\n")
            );
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

pub struct Client {
    pub ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub screen: Screen,
    /// The last frame received, without its rows.
    pub frame: Option<Frame>,
    /// Clipboard acks seen while waiting for something else.
    pub acks: VecDeque<ClipboardAck>,
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
        Some(Self {
            ws,
            screen: Screen::default(),
            frame: None,
            acks: VecDeque::new(),
        })
    }

    pub async fn send(&mut self, msg: Msg) {
        let packet = tilr_proto::encode(&ClientMessage::from(msg));
        self.ws.send(Message::Binary(packet)).await.expect("send");
    }

    /// The next message, or `None` once the server closed the connection.
    pub async fn next(&mut self) -> Option<server_message::Msg> {
        loop {
            let message = tokio::time::timeout(TIMEOUT, self.ws.next())
                .await
                .expect("timed out waiting for the server");
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
        self.send(Msg::Hello(Hello {
            protocol_version,
            client_name: "test".into(),
            features: vec!["clipboard".into()],
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
pub async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..50 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("did not happen: {what}");
}

/// Ask the server to reload and connect again once it has.
pub async fn reload(server: &Server, client: Client) -> Client {
    server.signal("-USR1");
    reconnect(server, client).await
}
