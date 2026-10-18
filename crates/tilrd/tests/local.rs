//! Use cases: the Unix socket for local clients (`tilr`): private to the
//! user, one server per socket, what is found at its path, reloads.

mod common;

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::process::CommandExt;

use common::*;
use tokio::net::UnixStream;

/// A server started as `tilr` starts one: the Unix socket, with or without
/// a TCP port.
struct Daemon {
    child: Child,
    dir: PathBuf,
    socket: PathBuf,
    log: Arc<Mutex<Vec<String>>>,
}

impl Daemon {
    /// Start `tilrd ARGS` with its directories under `dir`; the socket in
    /// its runtime directory unless `args` name one.
    fn spawn(dir: &Path, args: &[&str]) -> Self {
        Self::spawn_exe(Path::new(env!("CARGO_BIN_EXE_tilrd")), dir, args)
    }

    fn spawn_exe(exe: &Path, dir: &Path, args: &[&str]) -> Self {
        Self::spawn_env(exe, dir, args, &[])
    }

    /// With more variables in its environment.
    fn spawn_env(exe: &Path, dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Self {
        let mut command = Command::new(exe);
        command.args(args).envs(env.iter().copied());
        Server::environment(&mut command, dir, "");
        // The TCP port a server has by default: not the one of the machine.
        command.env("TILR_LISTEN", "127.0.0.1:0");
        command.stdin(Stdio::null()).stderr(Stdio::piped());
        // The executable may be a copy the test just made.
        let mut child = spawn_copied(&mut command);
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                sink.lock().unwrap().push(line);
            }
        });
        let socket = match args.iter().position(|arg| *arg == "--socket") {
            Some(at) => PathBuf::from(args[at + 1]),
            None => dir.join("runtime/tilrd.sock"),
        };
        Self {
            child,
            dir: dir.to_owned(),
            socket,
            log,
        }
    }

    /// Started and taking connections on its socket.
    fn start(dir: &Path, args: &[&str]) -> Self {
        let mut daemon = Self::spawn(dir, args);
        daemon.wait_listening();
        daemon
    }

    fn wait_listening(&mut self) {
        for _ in 0..1000 {
            if std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
                return;
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("tilrd exited {status}: {}", self.log_text());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "tilrd does not listen on {:?}: {}",
            self.socket,
            self.log_text()
        );
    }

    /// Wait for it to exit; its status and log.
    fn exit(&mut self) -> (std::process::ExitStatus, String) {
        for _ in 0..1000 {
            if let Some(status) = self.child.try_wait().unwrap() {
                // The log thread has read what was written before the exit.
                std::thread::sleep(Duration::from_millis(50));
                return (status, self.log_text());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("tilrd still runs: {}", self.log_text());
    }

    fn log_text(&self) -> String {
        self.log.lock().unwrap().join("\n")
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn signal(&self, signal: libc::c_int) {
        // SAFETY: kill has no memory-safety preconditions.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, signal);
        }
    }

    /// `tilrd ps` without --url: through the socket.
    fn ps(&self) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tilrd"));
        command.arg("ps");
        Server::environment(&mut command, &self.dir, "");
        command.env("TILR_SOCKET", &self.socket).output().unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if self.running() {
            self.signal(libc::SIGTERM);
            for _ in 0..100 {
                if !self.running() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            eprintln!("--- server log ---\n{}", self.log_text());
        }
    }
}

/// A directory of the test's own, removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        Self(Server::scratch_dir())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A WebSocket on the Unix socket, past `Hello` (no token).
async fn connect(socket: &Path) -> WebSocketStream<UnixStream> {
    let stream = UnixStream::connect(socket).await.expect("connect");
    let (mut ws, _) = tokio_tungstenite::client_async(tilr_proto::local::URL, stream)
        .await
        .expect("handshake");
    send(
        &mut ws,
        Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "test".into(),
            ..Default::default()
        }),
    )
    .await;
    match next(&mut ws).await {
        Some(server_message::Msg::HelloAck(_)) => ws,
        other => panic!("expected HelloAck, got {other:?}"),
    }
}

async fn send(ws: &mut WebSocketStream<UnixStream>, msg: Msg) {
    let packet = tilr_proto::encode(&ClientMessage::from(msg));
    ws.send(Message::Binary(packet)).await.expect("send");
}

async fn next(ws: &mut WebSocketStream<UnixStream>) -> Option<server_message::Msg> {
    loop {
        let message = tokio::time::timeout(TIMEOUT, ws.next())
            .await
            .expect("timed out waiting for the server");
        match message {
            Some(Ok(Message::Binary(packet))) => {
                if let Some(msg) = tilr_proto::decode::<ServerMessage>(&packet).unwrap().msg {
                    return Some(msg);
                }
            }
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
            Some(Ok(_)) => {}
        }
    }
}

async fn panes(ws: &mut WebSocketStream<UnixStream>) -> Vec<PaneInfo> {
    send(ws, Msg::ListPanes(ListPanes { request_id: 77 })).await;
    loop {
        if let Some(server_message::Msg::PaneList(list)) = next(ws).await
            && list.request_id == 77
        {
            return list.panes;
        }
    }
}

async fn create_pane(ws: &mut WebSocketStream<UnixStream>) -> String {
    send(
        ws,
        Msg::CreatePane(CreatePane {
            cols: 80,
            rows: 24,
            request_id: 5,
            ..Default::default()
        }),
    )
    .await;
    loop {
        match next(ws).await {
            Some(server_message::Msg::PaneCreated(created)) if created.request_id == 5 => {
                return created.pane.unwrap().pane_id;
            }
            Some(server_message::Msg::Error(err)) => panic!("create pane: {}", err.message),
            Some(_) => {}
            None => panic!("connection closed"),
        }
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn the_socket_is_the_users_alone_and_asks_no_token() {
    let scratch = Scratch::new();
    let daemon = Daemon::start(&scratch.0, &[]);
    assert_eq!(mode(daemon.socket.parent().unwrap()), 0o700);
    assert_eq!(mode(&daemon.socket), 0o600);
    let lock = tilr_proto::local::lock_path(&daemon.socket);
    assert_eq!(mode(&lock), 0o600);
    assert_eq!(
        std::fs::read_to_string(&lock).unwrap().trim(),
        daemon.child.id().to_string(),
        "the lock names its holder"
    );
    // A server with a token (it made one) takes a Hello without it here.
    let mut ws = connect(&daemon.socket).await;
    let pane = create_pane(&mut ws).await;
    assert!(panes(&mut ws).await.iter().any(|p| p.pane_id == pane));
    let ps = daemon.ps();
    assert!(ps.status.success(), "{ps:?}");
    assert!(String::from_utf8_lossy(&ps.stdout).contains(&pane));
    // And a TCP port, on loopback, by default.
    assert!(daemon.log_text().contains("listening on ws://127.0.0.1:"));
}

#[test]
fn a_service_manager_is_told_when_the_server_is_ready() {
    // systemd's Type=notify: READY=1 on $NOTIFY_SOCKET once it listens.
    let scratch = Scratch::new();
    let notify = scratch.0.join("notify");
    let manager = std::os::unix::net::UnixDatagram::bind(&notify).unwrap();
    manager
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut daemon = Daemon::spawn_env(
        Path::new(env!("CARGO_BIN_EXE_tilrd")),
        &scratch.0,
        &[],
        &[("NOTIFY_SOCKET", &notify)],
    );
    let mut message = [0; 64];
    let n = manager.recv(&mut message).expect("READY=1");
    assert_eq!(&message[..n], b"READY=1");
    daemon.wait_listening();
}

#[test]
fn no_listen_serves_the_socket_alone() {
    let scratch = Scratch::new();
    let daemon = Daemon::start(&scratch.0, &["--no-listen"]);
    assert!(daemon.ps().status.success());
    let log = daemon.log_text();
    assert!(!log.contains("listening on ws://"), "{log}");
    assert!(log.contains("no TCP port"), "{log}");
    let mut both = Daemon::spawn(&scratch.0, &["--no-listen", "--no-socket"]);
    let (status, log) = both.exit();
    assert!(!status.success());
    assert!(log.contains("nothing to listen on"), "{log}");
    daemon.signal(libc::SIGTERM);
}

#[test]
fn a_tcp_port_that_is_taken_stops_the_server_with_a_clear_error() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap().to_string();
    let scratch = Scratch::new();
    let mut daemon = Daemon::spawn(&scratch.0, &["--listen", &addr]);
    let (status, log) = daemon.exit();
    assert_eq!(status.code(), Some(1), "{log}");
    assert!(
        log.contains(&format!(
            "{addr} is taken: another tilrd, or another program, listens there"
        )) && log.contains("--no-listen"),
        "{log}"
    );
    assert!(!daemon.socket.exists(), "the socket is not left behind");
}

#[test]
fn another_user_is_turned_away_even_where_the_permissions_let_it_in() {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipped: needs root to act as another user (the itests run it, UC-034)");
        return;
    }
    let scratch = Scratch::new();
    let daemon = Daemon::start(&scratch.0, &[]);
    let as_nobody = |dir: &Path| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tilrd"));
        command.arg("ps");
        Server::environment(&mut command, dir, "");
        command.env("TILR_SOCKET", &daemon.socket);
        // SAFETY: setgid and setuid are async-signal-safe.
        unsafe {
            command.pre_exec(|| {
                if libc::setgid(65534) != 0 || libc::setuid(65534) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.output().unwrap()
    };
    let refused = as_nobody(&scratch.0);
    assert!(!refused.status.success(), "the directory keeps others out");
    // Opened up by hand: the server still asks who it is.
    for dir in [&scratch.0, &scratch.0.join("runtime")] {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::set_permissions(&daemon.socket, std::fs::Permissions::from_mode(0o666)).unwrap();
    let refused = as_nobody(&scratch.0);
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        daemon
            .log_text()
            .contains("refused a connection on the Unix socket")
    );
    assert!(daemon.ps().status.success(), "the user still gets in");
}

#[test]
fn a_socket_left_by_a_killed_server_is_replaced() {
    let scratch = Scratch::new();
    let mut first = Daemon::start(&scratch.0, &[]);
    first.signal(libc::SIGKILL);
    first.exit();
    let meta = std::fs::symlink_metadata(&first.socket).unwrap();
    assert!(meta.file_type().is_socket(), "SIGKILL leaves the socket");

    let second = Daemon::start(&scratch.0, &[]);
    assert!(
        second
            .log_text()
            .contains("removing the socket of a server that is gone"),
        "{}",
        second.log_text()
    );
    assert!(second.ps().status.success());
}

#[test]
fn a_file_at_the_path_is_left_alone_with_a_clear_error() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("runtime/tilrd.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    std::fs::write(&socket, "mine").unwrap();
    let mut daemon = Daemon::spawn(&scratch.0, &["--no-listen"]);
    let (status, log) = daemon.exit();
    assert!(!status.success());
    assert!(
        log.contains("it is not a socket (nothing was removed)"),
        "{log}"
    );
    assert_eq!(std::fs::read_to_string(&socket).unwrap(), "mine");
}

#[test]
fn a_symlinked_lock_file_is_refused() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("runtime/tilrd.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let target = scratch.0.join("elsewhere");
    std::fs::write(&target, "mine").unwrap();
    std::os::unix::fs::symlink(&target, tilr_proto::local::lock_path(&socket)).unwrap();
    let mut daemon = Daemon::spawn(&scratch.0, &["--no-listen"]);
    let (status, log) = daemon.exit();
    assert!(!status.success());
    assert!(log.contains("a symbolic link is not followed"), "{log}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
    assert!(!socket.exists(), "{log}");
}

#[test]
fn another_programs_listener_at_the_path_is_left_alone_with_a_clear_error() {
    let scratch = Scratch::new();
    let socket = scratch.0.join("runtime/tilrd.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let foreign = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let mut daemon = Daemon::spawn(&scratch.0, &["--no-listen"]);
    let (status, log) = daemon.exit();
    assert!(!status.success());
    assert!(
        log.contains("something that is not a tilrd listens"),
        "{log}"
    );
    // Still the other program's.
    let _client = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    foreign.set_nonblocking(true).unwrap();
    assert!(foreign.accept().is_ok());
}

#[test]
fn a_socket_path_too_long_is_a_clear_error() {
    let scratch = Scratch::new();
    let long = scratch.0.join(format!("{}/tilrd.sock", "d".repeat(120)));
    let long = long.to_str().unwrap();
    let mut daemon = Daemon::spawn(&scratch.0, &["--socket", long]);
    let (status, log) = daemon.exit();
    assert!(!status.success());
    assert!(log.contains("longer than a Unix socket's"), "{log}");
    assert!(log.contains("TILR_SOCKET"), "{log}");
}

#[test]
fn of_servers_started_at_once_one_serves_the_socket() {
    let scratch = Scratch::new();
    let mut daemons: Vec<Daemon> = (0..8)
        .map(|_| Daemon::spawn(&scratch.0, &["--no-listen"]))
        .collect();
    std::thread::sleep(Duration::from_millis(1500));
    let running: Vec<usize> = (0..daemons.len())
        .filter(|&i| daemons[i].running())
        .collect();
    assert_eq!(running.len(), 1, "exactly one stays");
    for (i, daemon) in daemons.iter_mut().enumerate() {
        if i != running[0] {
            let (status, log) = daemon.exit();
            assert!(!status.success());
            assert!(log.contains("another tilrd"), "{log}");
        }
    }
    let lock = tilr_proto::local::lock_path(&daemons[0].socket);
    assert_eq!(
        std::fs::read_to_string(lock).unwrap().trim(),
        daemons[running[0]].child.id().to_string()
    );
    assert!(daemons[running[0]].ps().status.success());
}

#[test]
fn with_a_tcp_port_a_socket_in_use_is_only_a_warning() {
    // The Tilr app starts `tilrd --listen …` whether or not a tilr started
    // a server for the socket first.
    let scratch = Scratch::new();
    let first = Daemon::start(&scratch.0, &[]);
    // A socket asked for by name (--socket) must work.
    let mut named = Daemon::spawn(
        &scratch.0,
        &[
            "--listen",
            "127.0.0.1:0",
            "--socket",
            first.socket.to_str().unwrap(),
        ],
    );
    let (status, log) = named.exit();
    assert!(
        !status.success(),
        "a socket asked for by name must work: {log}"
    );

    let mut tcp = Daemon::spawn(&scratch.0, &["--listen", "127.0.0.1:0"]);
    for _ in 0..500 {
        if tcp.log_text().contains("listening on ws://") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(tcp.running(), "{}", tcp.log_text());
    assert!(
        tcp.log_text()
            .contains("no Unix socket for local clients: another tilrd"),
        "{}",
        tcp.log_text()
    );
    assert!(first.ps().status.success(), "the first still serves it");
}

#[tokio::test]
async fn a_reload_keeps_the_socket_and_its_lock() {
    let scratch = Scratch::new();
    let mut daemon = Daemon::start(&scratch.0, &[]);
    let mut ws = connect(&daemon.socket).await;
    let pane = create_pane(&mut ws).await;
    drop(ws);
    daemon.signal(libc::SIGUSR1);
    for _ in 0..500 {
        if daemon.log_text().contains("resumed after reload") {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        daemon
            .log_text()
            .contains("resumed after reload with 1 pane(s)")
    );
    assert!(daemon.running(), "the same process");
    let mut ws = connect(&daemon.socket).await;
    assert!(panes(&mut ws).await.iter().any(|p| p.pane_id == pane));
    // Still the one server: another cannot take the socket.
    let mut other = Daemon::spawn(&scratch.0, &["--no-listen"]);
    let (status, log) = other.exit();
    assert!(!status.success());
    assert!(
        log.contains(&format!("another tilrd (pid {})", daemon.child.id())),
        "{log}"
    );
}

#[test]
fn a_server_that_stops_removes_its_socket_and_lock() {
    let scratch = Scratch::new();
    let mut daemon = Daemon::start(&scratch.0, &[]);
    daemon.signal(libc::SIGTERM);
    let (status, _) = daemon.exit();
    assert!(status.success());
    assert!(!daemon.socket.exists());
    assert!(!tilr_proto::local::lock_path(&daemon.socket).exists());
}

/// A server started with a pause between serving its socket and installing
/// its signal handlers (the failpoint `supervisor-before-signals`), once its
/// socket takes connections: the moment a signal used to kill it.
#[cfg(feature = "failpoints")]
fn starting_with_a_window(scratch: &Scratch) -> Daemon {
    std::fs::write(
        scratch.0.join("failpoints"),
        "supervisor-before-signals=sleep:1000",
    )
    .unwrap();
    Daemon::start(&scratch.0, &[])
}

#[cfg(feature = "failpoints")]
fn stops_cleanly_on(signal: libc::c_int, name: &str) {
    let scratch = Scratch::new();
    let mut daemon = starting_with_a_window(&scratch);
    daemon.signal(signal);
    let (status, log) = daemon.exit();
    assert!(
        status.success(),
        "a clean stop, not the signal: {status}\n{log}"
    );
    assert!(
        log.contains(&format!("{name} came while the server started")),
        "{log}"
    );
    assert!(log.contains("shutting down"), "{log}");
    assert!(!daemon.socket.exists(), "the socket is left");
    assert!(
        !tilr_proto::local::lock_path(&daemon.socket).exists(),
        "the lock file is left"
    );
}

#[cfg(feature = "failpoints")]
#[test]
fn a_sigterm_while_the_server_starts_stops_it_cleanly() {
    stops_cleanly_on(libc::SIGTERM, "SIGTERM");
}

#[cfg(feature = "failpoints")]
#[test]
fn a_sigint_while_the_server_starts_stops_it_cleanly() {
    stops_cleanly_on(libc::SIGINT, "SIGINT");
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_sighup_while_the_server_starts_is_ignored() {
    let scratch = Scratch::new();
    let mut daemon = starting_with_a_window(&scratch);
    daemon.signal(libc::SIGHUP);
    wait_logged(&daemon, "SIGHUP ignored");
    assert!(daemon.running(), "the server keeps running");
    let mut ws = connect(&daemon.socket).await;
    let pane = create_pane(&mut ws).await;
    assert!(panes(&mut ws).await.iter().any(|p| p.pane_id == pane));
}

#[tokio::test]
async fn server_commands_run_for_local_clients_only() {
    let scratch = Scratch::new();
    let daemon = Daemon::start(&scratch.0, &["--listen", "127.0.0.1:0", "--no-exec"]);
    let token = std::fs::read_to_string(scratch.0.join("state/token")).unwrap();

    // On the socket, even with exec off: `tilr token`.
    let mut ws = connect(&daemon.socket).await;
    send(
        &mut ws,
        Msg::ExecStart(tilr_proto::ExecStart {
            request_id: 1,
            argv: vec!["token".into()],
            server_command: true,
            ..Default::default()
        }),
    )
    .await;
    let mut out = Vec::new();
    loop {
        match next(&mut ws).await.expect("connection") {
            server_message::Msg::ExecStarted(started) => assert_eq!(started.error, ""),
            server_message::Msg::ExecOutput(output) => out.extend_from_slice(&output.data),
            server_message::Msg::ExecExit(exit) => {
                assert_eq!(exit.code, 0);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(String::from_utf8_lossy(&out), token);

    // On TCP, with the token: refused.
    let addr = daemon
        .log_text()
        .lines()
        .find_map(|line| line.split("listening on ").nth(1).map(str::to_owned))
        .unwrap();
    let mut client = Client::try_connect(&Server {
        child: Command::new("true").spawn().unwrap(),
        addr: addr.trim().to_owned(),
        dir: scratch.0.clone(),
        log: Arc::default(),
    })
    .await
    .unwrap();
    client
        .send(Msg::ExecStart(tilr_proto::ExecStart {
            request_id: 2,
            argv: vec!["token".into()],
            server_command: true,
            ..Default::default()
        }))
        .await;
    let started = client
        .wait_for(|msg| match msg {
            server_message::Msg::ExecStarted(started) => Some(started.clone()),
            _ => None,
        })
        .await;
    assert!(
        started.error.contains("Unix socket only"),
        "{}",
        started.error
    );
}

/// An executable that reads the state but not the socket (no `unix-socket
/// 1`) in place of `exe`.
fn replace_with_one_that_cannot_take_the_socket(scratch: &Scratch, exe: &Path) {
    let formats = Command::new(exe).arg("state-formats").output().unwrap();
    let formats = String::from_utf8(formats.stdout).unwrap();
    let older: String = formats
        .lines()
        .filter(|line| !line.starts_with("unix-socket"))
        .map(|line| format!("{line}\n"))
        .collect();
    let staged = scratch.0.join("tilrd.new");
    std::fs::write(
        &staged,
        format!("#!/bin/sh\nprintf '{}'\n", older.replace('\n', "\\n")),
    )
    .unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, exe).unwrap();
}

fn wait_logged(daemon: &Daemon, needle: &str) {
    for _ in 0..500 {
        if daemon.log_text().contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("not logged: {needle}\n{}", daemon.log_text());
}

#[test]
fn a_server_on_its_socket_alone_does_not_reload_into_one_that_cannot_take_it() {
    // An older tilrd reads the same state but knows nothing of the socket.
    let scratch = Scratch::new();
    let exe = scratch.0.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let mut daemon = Daemon::spawn_exe(&exe, &scratch.0, &["--no-listen"]);
    daemon.wait_listening();
    replace_with_one_that_cannot_take_the_socket(&scratch, &exe);
    wait_logged(
        &daemon,
        "cannot take over the Unix socket this server serves",
    );
    assert!(daemon.running());
    assert!(daemon.ps().status.success());
}

/// With a TCP port too, the server used to reload into it serving TCP
/// alone, leaving the socket file with nobody behind it: the next `tilr`
/// started a second server.
#[test]
fn a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket() {
    let scratch = Scratch::new();
    let exe = scratch.0.join("tilrd");
    std::fs::copy(env!("CARGO_BIN_EXE_tilrd"), &exe).unwrap();
    let mut daemon = Daemon::spawn_exe(&exe, &scratch.0, &[]);
    daemon.wait_listening();
    replace_with_one_that_cannot_take_the_socket(&scratch, &exe);
    wait_logged(
        &daemon,
        "cannot take over the Unix socket this server serves",
    );
    assert!(!daemon.log_text().contains("resumed after reload"));
    assert!(daemon.running());
    assert!(daemon.ps().status.success(), "served through the socket");
}

/// A reload refused by the check of the successor leaves the socket, its
/// lock and the connections on it as they were.
#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_refused_reload_keeps_the_socket_and_its_lock() {
    let scratch = Scratch::new();
    let mut daemon = Daemon::start(&scratch.0, &[]);
    let mut ws = connect(&daemon.socket).await;
    let pane = create_pane(&mut ws).await;
    std::fs::write(scratch.0.join("failpoints"), "supervisor-resume=panic").unwrap();
    daemon.signal(libc::SIGUSR1);
    wait_logged(
        &daemon,
        "not reloading: the new executable could not resume",
    );
    assert!(daemon.running(), "the same process");
    // The same connection, and a new one through the socket.
    assert!(panes(&mut ws).await.iter().any(|p| p.pane_id == pane));
    let mut again = connect(&daemon.socket).await;
    assert!(panes(&mut again).await.iter().any(|p| p.pane_id == pane));
    assert!(daemon.ps().status.success());
    // Still locked: another server cannot take the socket.
    let mut other = Daemon::spawn(&scratch.0, &["--no-listen"]);
    let (status, log) = other.exit();
    assert!(!status.success());
    assert!(
        log.contains(&format!("another tilrd (pid {})", daemon.child.id())),
        "{log}"
    );
}
