//! End-to-end use cases: each test starts the real server, talks to it over
//! WebSocket like a client would, and looks at what the shell in a pane sees.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tilr_proto::{
    ClientMessage, ClipboardAck, ClipboardPut, ClosePane, CreatePane, Frame, Hello, HelloAck,
    ListPanes, PROTOCOL_VERSION, PaneInfo, RawInput, ServerMessage, Subscribe, client_message::Msg,
    server_message,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const TIMEOUT: Duration = Duration::from_secs(20);
const CHUNK: usize = 256 * 1024;

/// A running server with a scratch directory of its own.
struct Server {
    child: Child,
    addr: String,
    /// Scratch directory: the server's runtime directory lives in it, and
    /// tests put their own files there.
    dir: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(args: &[&str]) -> Self {
        Self::start_exe(Path::new(env!("CARGO_BIN_EXE_tilr-server")), args)
    }

    fn scratch_dir() -> PathBuf {
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

    fn start_exe(exe: &Path, args: &[&str]) -> Self {
        Self::start_in(Self::scratch_dir(), exe, args)
    }

    fn start_in(dir: PathBuf, exe: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(exe)
            .args(["--listen", "127.0.0.1:0"])
            .args(args)
            .env("TILR_RUNTIME_DIR", dir.join("runtime"))
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .env("PS1", "$ ")
            // No real clipboard for the shims to fall back to, and no shell
            // startup file.
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("ENV")
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start server");

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

    fn logged(&self, needle: &str) -> bool {
        self.log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(needle))
    }

    fn signal(&self, signal: &str) {
        let status = Command::new("kill")
            .args([signal, &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn is_running(&mut self) -> bool {
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
struct Screen {
    rows: Vec<String>,
}

impl Screen {
    fn apply(&mut self, frame: &Frame) {
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

    fn text(&self) -> String {
        self.rows.join("\n")
    }
}

struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    screen: Screen,
    /// Clipboard acks seen while waiting for something else.
    acks: VecDeque<ClipboardAck>,
}

impl Client {
    async fn connect(server: &Server) -> Self {
        Self::try_connect(server).await.expect("connect")
    }

    async fn try_connect(server: &Server) -> Option<Self> {
        let (ws, _) = tokio_tungstenite::connect_async(&server.addr).await.ok()?;
        Some(Self {
            ws,
            screen: Screen::default(),
            acks: VecDeque::new(),
        })
    }

    async fn send(&mut self, msg: Msg) {
        let packet = tilr_proto::encode(&ClientMessage::from(msg));
        self.ws.send(Message::Binary(packet)).await.expect("send");
    }

    /// The next message, or `None` once the server closed the connection.
    async fn next(&mut self) -> Option<server_message::Msg> {
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
                        Some(server_message::Msg::Frame(frame)) => self.screen.apply(frame),
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
    async fn wait_for<T>(&mut self, mut pick: impl FnMut(&server_message::Msg) -> Option<T>) -> T {
        loop {
            let msg = self.next().await.expect("connection closed");
            if let Some(picked) = pick(&msg) {
                return picked;
            }
        }
    }

    async fn hello(&mut self) -> HelloAck {
        self.send(Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "test".into(),
            features: vec!["clipboard".into()],
        }))
        .await;
        self.wait_for(|msg| match msg {
            server_message::Msg::HelloAck(ack) => Some(ack.clone()),
            _ => None,
        })
        .await
    }

    async fn list_panes(&mut self) -> Vec<PaneInfo> {
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
    async fn open_pane(&mut self) -> u64 {
        self.send(Msg::CreatePane(CreatePane {
            cols: 100,
            rows: 30,
            command: vec!["/bin/sh".into()],
            cwd: String::new(),
            request_id: 5,
        }))
        .await;
        let pane_id = self
            .wait_for(|msg| match msg {
                server_message::Msg::PaneCreated(created) if created.request_id == 5 => {
                    Some(created.pane.as_ref().unwrap().pane_id)
                }
                server_message::Msg::Error(error) => panic!("create pane: {}", error.message),
                _ => None,
            })
            .await;
        self.subscribe(pane_id).await;
        pane_id
    }

    async fn subscribe(&mut self, pane_id: u64) {
        self.screen = Screen::default();
        self.send(Msg::Subscribe(Subscribe { pane_id })).await;
    }

    /// Type a line into the pane's shell.
    async fn type_line(&mut self, pane_id: u64, line: &str) {
        self.send(Msg::Raw(RawInput {
            pane_id,
            data: Bytes::from(format!("{line}\n")),
        }))
        .await;
    }

    /// Wait until the pane's screen shows `text`.
    async fn wait_screen(&mut self, text: &str) {
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
    async fn run(&mut self, pane_id: u64, command: &str) -> i32 {
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
    async fn put_clipboard(&mut self, transfer_id: u64, mime: &str, data: &[u8]) {
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
    async fn clipboard_ack(&mut self) -> ClipboardAck {
        loop {
            if let Some(ack) = self.acks.pop_front() {
                return ack;
            }
            self.next().await.expect("connection closed");
        }
    }
}

/// Bytes that look like nothing in particular, newlines and zeros included.
fn image(len: usize) -> Vec<u8> {
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
const CLAUDE_CHECK_IMAGE: &str = r#"xclip -selection clipboard -t TARGETS -o 2>/dev/null | grep -E "image/(png|jpeg|jpg|gif|webp|bmp)" || wl-paste -l 2>/dev/null | grep -E "image/(png|jpeg|jpg|gif|webp|bmp)""#;

fn claude_save_image(file: &Path) -> String {
    let file = file.display();
    format!(
        "xclip -selection clipboard -t image/png -o > '{file}' 2>/dev/null || wl-paste --type image/png > '{file}' 2>/dev/null || xclip -selection clipboard -t image/bmp -o > '{file}' 2>/dev/null || wl-paste --type image/bmp > '{file}'"
    )
}

/// Write `script` to a file in the server's scratch directory and return the
/// command that runs it. Keeps long commands off the pane's command line.
fn script(server: &Server, name: &str, script: &str) -> String {
    let path = server.dir.join(name);
    std::fs::write(&path, script).unwrap();
    format!("sh '{}'", path.display())
}

#[tokio::test]
async fn hello_announces_the_clipboard_and_its_limit() {
    let server = Server::start(&["--clipboard-max", "3MB"]);
    let mut client = Client::connect(&server).await;
    let ack = client.hello().await;
    assert_eq!(ack.protocol_version, PROTOCOL_VERSION);
    assert_eq!(ack.features, ["clipboard"]);
    assert_eq!(ack.clipboard_max_bytes, 3 << 20);
}

#[tokio::test]
async fn a_pane_runs_commands_and_reports_how_its_shell_exited() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    client.hello().await;
    assert!(client.list_panes().await.is_empty());

    let pane_id = client.open_pane().await;
    client
        .type_line(pane_id, "echo \"sum is $((6 * 7))\"")
        .await;
    client.wait_screen("sum is 42").await;

    let panes = client.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, pane_id);
    assert_eq!(panes[0].viewers, 1);
    assert_eq!((panes[0].cols, panes[0].rows), (100, 30));

    client.type_line(pane_id, "exit 3").await;
    let exit_code = client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneExited(exited) if exited.pane_id == pane_id => {
                Some(exited.exit_code)
            }
            _ => None,
        })
        .await;
    assert_eq!(exit_code, 3);
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn closing_a_pane_ends_its_shell() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client.send(Msg::ClosePane(ClosePane { pane_id })).await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::PaneExited(exited) if exited.pane_id == pane_id => Some(()),
            _ => None,
        })
        .await;
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn a_message_type_the_server_does_not_know_is_ignored() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Flag byte, then field 99 of ClientMessage holding an empty message.
    let unknown = Bytes::from_static(&[0, 0x9a, 0x06, 0x00]);
    client.ws.send(Message::Binary(unknown)).await.unwrap();
    client
        .send(Msg::ListPanes(ListPanes { request_id: 1 }))
        .await;
    // The very next message answers the request: no error came before it.
    assert!(matches!(
        client.next().await,
        Some(server_message::Msg::PaneList(list)) if list.request_id == 1
    ));
}

#[tokio::test]
async fn a_pasted_image_reaches_a_program_that_reads_the_clipboard() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    client.hello().await;
    let pane_id = client.open_pane().await;

    // Nothing was pasted yet: Claude Code finds no image.
    let check = script(&server, "check.sh", CLAUDE_CHECK_IMAGE);
    assert_ne!(client.run(pane_id, &check).await, 0);

    // Large enough to travel in several chunks.
    let data = image(2 * CHUNK + 12_345);
    client.put_clipboard(1, "image/png", &data).await;
    // The client does not wait for the ack: what it sends next is handled
    // after the upload, so the paste keystroke would find the image.
    assert_eq!(client.run(pane_id, &check).await, 0);
    let ack = client.clipboard_ack().await;
    assert!(ack.ok, "{}", ack.error);
    assert_eq!(ack.transfer_id, 1);

    let saved = server.dir.join("claude_cli_latest_screenshot.png");
    let save = script(&server, "save.sh", &claude_save_image(&saved));
    assert_eq!(client.run(pane_id, &save).await, 0);
    assert_eq!(std::fs::read(&saved).unwrap(), data);

    // The same through wl-paste alone.
    let listed = server.dir.join("types");
    let list = format!("wl-paste -l > '{}'", listed.display());
    assert_eq!(client.run(pane_id, &list).await, 0);
    assert_eq!(std::fs::read_to_string(&listed).unwrap(), "image/png\n");
    let pasted = server.dir.join("wl.png");
    let paste = format!("wl-paste --type image/png > '{}'", pasted.display());
    assert_eq!(client.run(pane_id, &paste).await, 0);
    assert_eq!(std::fs::read(&pasted).unwrap(), data);

    // An image is not text: a program asking for text gets nothing.
    assert_ne!(
        client.run(pane_id, "xclip -selection clipboard -o").await,
        0
    );
}

#[tokio::test]
async fn the_clipboard_is_shared_by_all_panes_and_replaced_by_the_next_paste() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let first = client.open_pane().await;
    client
        .put_clipboard(1, "text/plain", b"from the first paste")
        .await;
    assert!(client.clipboard_ack().await.ok);

    // A pane opened later, watched by another client, sees the same content.
    let mut other = Client::connect(&server).await;
    let second = other.open_pane().await;
    let out = server.dir.join("second.txt");
    let read = format!("xclip -selection clipboard -o > '{}'", out.display());
    assert_eq!(other.run(second, &read).await, 0);
    assert_eq!(std::fs::read(&out).unwrap(), b"from the first paste");

    other.put_clipboard(1, "text/plain", b"replaced").await;
    assert!(other.clipboard_ack().await.ok);
    let out = server.dir.join("first.txt");
    let read = format!("wl-paste --no-newline > '{}'", out.display());
    assert_eq!(client.run(first, &read).await, 0);
    assert_eq!(std::fs::read(&out).unwrap(), b"replaced");
}

#[tokio::test]
async fn an_upload_over_the_limit_is_refused_and_leaves_no_stale_content() {
    let server = Server::start(&["--clipboard-max", "64K"]);
    let mut client = Client::connect(&server).await;
    assert_eq!(client.hello().await.clipboard_max_bytes, 64 << 10);
    let pane_id = client.open_pane().await;
    let check = script(&server, "check.sh", CLAUDE_CHECK_IMAGE);

    client.put_clipboard(1, "image/png", &image(1000)).await;
    assert!(client.clipboard_ack().await.ok);
    assert_eq!(client.run(pane_id, &check).await, 0);

    // A gigabyte is announced: the server says no on the first chunk.
    client
        .send(Msg::ClipboardPut(ClipboardPut {
            transfer_id: 2,
            mime: "image/png".into(),
            total_size: 1 << 30,
            data: Bytes::from(image(CHUNK)),
            last: false,
        }))
        .await;
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert_eq!(ack.transfer_id, 2);
    assert!(ack.error.contains("too large: 1 GB"), "{}", ack.error);
    assert!(ack.error.contains("at most 64 KB"), "{}", ack.error);
    // The image pasted before must not come back in its place.
    assert_ne!(client.run(pane_id, &check).await, 0);

    // Announcing less than is sent does not get around the limit.
    client.put_clipboard(3, "image/png", &image(1000)).await;
    assert!(client.clipboard_ack().await.ok);
    for last in [false, true] {
        client
            .send(Msg::ClipboardPut(ClipboardPut {
                transfer_id: 4,
                mime: if last {
                    String::new()
                } else {
                    "image/png".into()
                },
                total_size: if last { 0 } else { 100 },
                data: Bytes::from(image(80)),
                last,
            }))
            .await;
    }
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert_eq!(ack.transfer_id, 4);
    assert_ne!(client.run(pane_id, &check).await, 0);
}

#[tokio::test]
async fn the_clipboard_can_be_turned_off() {
    let server = Server::start(&["--no-clipboard"]);
    let mut client = Client::connect(&server).await;
    let ack = client.hello().await;
    assert!(ack.features.is_empty());
    assert_eq!(ack.clipboard_max_bytes, 0);

    client
        .put_clipboard(1, "image/png", &image(CHUNK + 1))
        .await;
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert!(ack.error.contains("turned off"), "{}", ack.error);

    // The panes are left alone: no tools of the server's on their PATH.
    let pane_id = client.open_pane().await;
    let out = server.dir.join("env.txt");
    let print = format!("echo \"$PATH|$TILR_CLIPBOARD_DIR\" > '{}'", out.display());
    assert_eq!(client.run(pane_id, &print).await, 0);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "/usr/bin:/bin|\n");
}

#[tokio::test]
async fn pasted_content_expires() {
    let server = Server::start(&["--clipboard-ttl", "1"]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let check = script(&server, "check.sh", CLAUDE_CHECK_IMAGE);
    client.put_clipboard(1, "image/png", &image(100)).await;
    assert!(client.clipboard_ack().await.ok);
    assert_eq!(client.run(pane_id, &check).await, 0);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_ne!(client.run(pane_id, &check).await, 0);
}

/// Wait for the server to drop the connection, then connect again.
async fn reconnect(server: &Server, mut client: Client) -> Client {
    while client.next().await.is_some() {}
    for _ in 0..100 {
        if let Some(client) = Client::try_connect(server).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("server did not come back");
}

/// What must hold after any reload: the shell in `pane_id` is the same one,
/// with its state and what it printed, and the server works as before.
async fn assert_pane_survived(server: &mut Server, mut client: Client, pane_id: u64) {
    assert!(server.is_running());
    client.hello().await;
    let panes = client.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, pane_id);
    assert_eq!((panes[0].cols, panes[0].rows), (100, 30));

    client.subscribe(pane_id).await;
    // The screen from before the reload is still there.
    client.wait_screen("before the reload").await;
    // A variable set before the reload is still set: same shell process.
    client
        .type_line(pane_id, "echo \"kept=$KEPT pid=$$\"")
        .await;
    client.wait_screen("kept=yes pid=").await;
    let pid_line = |text: String| {
        text.lines()
            .rev()
            .find(|line| line.contains("pid="))
            .map(str::to_owned)
    };
    assert_eq!(
        pid_line(client.screen.text()),
        pid_line(std::fs::read_to_string(server.dir.join("pid")).unwrap())
    );

    // New panes get ids after the old ones, and the clipboard still works.
    let new_pane = client.open_pane().await;
    assert!(new_pane > pane_id);
    client.put_clipboard(1, "image/png", &image(500)).await;
    assert!(client.clipboard_ack().await.ok);
    let check = script(server, "check.sh", CLAUDE_CHECK_IMAGE);
    assert_eq!(client.run(new_pane, &check).await, 0);
    assert!(server.logged("resumed after reload with 1 pane(s)"));
}

/// Open a pane and leave marks in its shell for [`assert_pane_survived`].
async fn pane_with_marks(server: &Server, client: &mut Client) -> u64 {
    let pane_id = client.open_pane().await;
    let pid_file = server.dir.join("pid");
    client
        .type_line(
            pane_id,
            &format!(
                "KEPT=yes; echo 'before the reload'; echo \"kept=$KEPT pid=$$\" > '{}'",
                pid_file.display()
            ),
        )
        .await;
    client.wait_screen("before the reload").await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    pane_id
}

#[tokio::test]
async fn panes_survive_a_reload_asked_for_with_a_signal() {
    let mut server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    server.signal("-USR1");
    let client = reconnect(&server, client).await;
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[tokio::test]
async fn panes_survive_the_executable_being_replaced_on_disk() {
    // Run a copy of the server, then install a "new version" over it the
    // way an upgrade does: write next to it, rename into place.
    let dir = Server::scratch_dir();
    let exe = dir.join("tilr-server");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &exe).unwrap();
    let mut server = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = pane_with_marks(&server, &mut client).await;

    let staged = dir.join("tilr-server.new");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &staged).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    // Nobody tells the server: it notices on its own.
    let client = reconnect(&server, client).await;
    assert!(server.logged("executable changed on disk"));
    assert_pane_survived(&mut server, client, pane_id).await;
}

#[tokio::test]
async fn a_replaced_executable_is_ignored_when_auto_reload_is_off() {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilr-server");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &exe).unwrap();
    let server = Server::start_in(dir.clone(), &exe, &["--no-auto-reload"]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;

    let staged = dir.join("tilr-server.new");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &staged).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    tokio::time::sleep(Duration::from_secs(4)).await;
    // Still the same connection, still the same server.
    assert_eq!(client.run(pane_id, "true").await, 0);
    assert!(!server.logged("reloading"));
}

#[tokio::test]
async fn a_broken_executable_is_not_reloaded_into() {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilr-server");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &exe).unwrap();
    let mut server = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;

    // An upgrade gone wrong: the new file does not run.
    let staged = dir.join("tilr-server.new");
    std::fs::write(&staged, "#!/bin/sh\nexit 1\n").unwrap();
    let mut mode = std::fs::metadata(&staged).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&staged, mode).unwrap();
    std::fs::rename(&staged, &exe).unwrap();

    for _ in 0..100 {
        if server.logged("not reloading") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(server.logged("not reloading"));
    // The server carries on with the code it has.
    assert!(server.is_running());
    assert_eq!(client.run(pane_id, "true").await, 0);
}
