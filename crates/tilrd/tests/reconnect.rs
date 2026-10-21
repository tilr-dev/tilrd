//! Use cases: a client goes away and comes back.

mod common;

use common::*;

#[tokio::test]
async fn a_client_that_reconnects_finds_its_pane_as_it_left_it() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client
        .type_line(pane_id, "KEPT=yes; echo \"left on the scr\"\"een\"")
        .await;
    client.wait_row("left on the screen").await;
    drop(client);

    let mut client = Client::connect(&server).await;
    client.hello().await;
    // The server may not have seen the old connection close yet.
    eventually_viewers(&mut client, 0).await;
    let panes = client.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, pane_id.to_string());
    assert_eq!(panes[0].viewers, 0);

    // The first frame after subscribing carries the whole screen.
    client.subscribe(pane_id).await;
    let frame = client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) => Some(frame.clone()),
            _ => None,
        })
        .await;
    assert!(frame.full);
    assert_eq!((frame.cols, frame.rows), (100, 30));
    assert!(client.screen.has_row("left on the screen"));

    // Same shell: its variable is still set.
    client.type_line(pane_id, "echo \"kept=$KEPT\"").await;
    client.wait_row("kept=yes").await;
}

#[tokio::test]
async fn output_printed_while_nobody_watched_is_there_on_reconnect() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client
        .type_line(pane_id, "sleep 1; echo \"printed la\"\"ter\"")
        .await;
    // Make sure the shell has the line before leaving.
    client.wait_screen("sleep 1").await;
    drop(client);

    tokio::time::sleep(Duration::from_millis(1500)).await;
    let mut client = Client::connect(&server).await;
    client.subscribe(pane_id).await;
    client.wait_row("printed later").await;
}

#[tokio::test]
async fn subscribing_again_sends_the_whole_screen_again() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client
        .type_line(pane_id, "echo \"already th\"\"ere\"")
        .await;
    client.wait_row("already there").await;

    // A client that lost track of the screen asks for it afresh.
    client.subscribe(pane_id).await;
    let frame = client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) => Some(frame.clone()),
            _ => None,
        })
        .await;
    assert!(frame.full);
    assert!(client.screen.has_row("already there"));
    assert_eq!(client.list_panes().await[0].viewers, 1);
}

#[tokio::test]
async fn many_reconnects_leave_no_viewers_behind() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    for _ in 0..5 {
        drop(client);
        client = Client::connect(&server).await;
        client.subscribe(pane_id).await;
        assert_eq!(client.run(pane_id, "true").await, 0);
    }
    // Only the connection that is still open counts.
    eventually_viewers(&mut client, 1).await;
}

async fn eventually_viewers(client: &mut Client, viewers: u32) {
    let mut seen = 0;
    for _ in 0..50 {
        seen = client.list_panes().await[0].viewers;
        if seen == viewers {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("viewers stayed at {seen}, expected {viewers}");
}

#[tokio::test]
async fn a_client_that_vanishes_without_closing_is_dropped() {
    let server = Server::start(&["--client-timeout", "1"]);
    let mut stays = Client::connect(&server).await;
    let pane_id = stays.open_pane().await;
    let mut vanishes = Client::connect(&server).await;
    vanishes.subscribe(pane_id).await;
    vanishes
        .wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;
    assert_eq!(stays.list_panes().await[0].viewers, 2);

    // The network drops out: the connection stays open, but `vanishes` no
    // longer reads it and so answers no ping. The server has to notice by
    // itself and tell the client that is still there.
    loop {
        let panes = stays.pushed_pane_list().await;
        if panes[0].viewers == 1 {
            break;
        }
    }
    server.wait_logged("client went silent").await;
    // The pane and the client that kept answering are unharmed.
    assert_eq!(stays.run(pane_id, "true").await, 0);

    // Back on the network, the client finds its connection closed, connects
    // again and has its pane back.
    let mut returns = reconnect(&server, vanishes).await;
    returns.subscribe(pane_id).await;
    assert_eq!(returns.run(pane_id, "true").await, 0);
    eventually_viewers(&mut returns, 2).await;
}

#[tokio::test]
async fn a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped() {
    let server = Server::start(&["--client-timeout", "1"]);
    let mut stays = Client::connect(&server).await;
    let pane_id = stays.open_pane().await;
    let mut deaf = Client::connect(&server).await;
    deaf.subscribe(pane_id).await;
    deaf.wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;
    while stays.pushed_pane_list().await[0].viewers != 2 {}

    // The way from the server to `deaf` is cut: it reads nothing, so it
    // answers no ping, but what it sends (its own pings) still arrives.
    let talking = async {
        for nonce in 0..u64::MAX {
            let ping = ClientMessage::from(Msg::Ping(Ping { nonce }));
            let _ = deaf
                .ws
                .send(Message::Binary(tilr_proto::encode(&ping)))
                .await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    let dropped = async { while stays.pushed_pane_list().await[0].viewers != 1 {} };
    tokio::select! {
        () = talking => unreachable!(),
        () = dropped => {}
    }
    server.wait_logged("client stopped answering pings").await;
    assert!(!server.logged("client went silent"));
    assert_eq!(stays.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn an_idle_client_that_is_still_there_stays_connected() {
    let server = Server::start(&["--client-timeout", "1"]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);

    // Nothing to say for three timeouts. Reading the connection is all it
    // takes to answer the server's pings.
    let idle = async {
        loop {
            assert!(client.next().await.is_some(), "connection closed");
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(3), idle).await;
    assert!(!server.logged("client went silent"));
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_silent_client_is_kept_when_the_timeout_is_off() {
    let server = Server::start(&["--client-timeout", "0"]);
    let mut stays = Client::connect(&server).await;
    let pane_id = stays.open_pane().await;
    let mut silent = Client::connect(&server).await;
    silent.subscribe(pane_id).await;
    silent
        .wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;

    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(stays.list_panes().await[0].viewers, 2);
    assert_eq!(silent.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_client_can_tell_that_the_server_vanished_without_closing() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Ping is part of the protocol: no feature announces it.
    assert!(!client.hello().await.features.contains(&"ping".to_owned()));
    let pane_id = client.open_pane().await;

    // A server that is there answers a ping at once.
    client.ping(1).await;
    assert!(client.pong_within(1, TIMEOUT).await);

    // The server freezes: the connection stays open and nothing comes over
    // it, which looks to the client just like a network that dropped out.
    server.signal("-STOP");
    client.ping(2).await;
    assert!(!client.pong_within(2, Duration::from_secs(1)).await);

    // Had it only been slow, the answer still comes and all is as it was.
    server.signal("-CONT");
    assert!(client.pong_within(2, TIMEOUT).await);
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_server_started_from_a_terminal_outlives_the_terminal_hanging_up() {
    // `tilrd &` typed into an SSH login; the dir goes to `Server`,
    // which removes it.
    let dir = Server::scratch_dir();
    let mut shell = Command::new("/bin/sh");
    shell
        .arg("-c")
        .arg("\"$0\" --listen 127.0.0.1:0 & echo \"server pid $!\"; wait")
        .arg(env!("CARGO_BIN_EXE_tilrd"));
    Server::environment(&mut shell, &dir, "");
    let owner = Owner::new(&dir);
    owner.configure(&mut shell);
    let mut login = Login::start(shell);
    let addr = login.wait_output(|output| {
        output
            .split("listening on ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .map(str::to_owned)
    });
    let pid: u32 = login.wait_output(|output| {
        output
            .split("server pid ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|pid| pid.parse().ok())
    });
    let mut server = Server {
        owner,
        child: login.shell.take().unwrap(),
        addr,
        dir,
        log: Arc::default(),
        log_changed: Arc::default(),
    };
    let _stop = StopOnDrop(pid);

    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let pane_pid = client.list_panes().await[0].pid;
    let shell_pid = shell_pid(&server, &mut client, pane_id).await;
    client.type_line(pane_id, "KEPT=yes").await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    drop(client);

    // The connection drops: the terminal hangs up, and its process group,
    // the server's, gets SIGHUP.
    login.hang_up();
    // A shell ending on a signal: gone at the first look (0.1 s) even with
    // four test suites running beside it, but once still there after 5 s
    // with a Linux VM and Dagger taking the machine. Nothing in the test or
    // the server was found to hold it: it waits to be scheduled.
    eventually_within("the login shell exits with its terminal", TIMEOUT, || {
        !server.is_running()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(process_alive(pid), "the server is still running");
    assert!(process_alive(pane_pid), "the pane process is still running");
    assert!(process_alive(shell_pid), "the shell is still running");

    let mut client = Client::connect(&server).await;
    client.subscribe(pane_id).await;
    client.type_line(pane_id, "echo \"kept=$KEPT\"").await;
    client.wait_row("kept=yes").await;

    // Stopping it still ends the shells.
    // SAFETY: kill has no memory-safety preconditions.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    eventually("the shell exits", || !process_alive(shell_pid)).await;
    eventually("the server exits", || !process_alive(pid)).await;
}

/// An SSH login: a shell leading a session of its own, with a pty as its
/// controlling terminal. What the terminal shows is collected until it hangs
/// up.
struct Login {
    shell: Option<Child>,
    /// The login shell's pid, which names its session and process group.
    pgid: u32,
    output: Arc<Mutex<String>>,
    hang_up: Arc<std::sync::atomic::AtomicBool>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Login {
    fn start(mut shell: Command) -> Self {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        use std::os::unix::process::CommandExt;

        let (mut master, mut slave) = (-1, -1);
        // SAFETY: out-pointers are valid; name, termios and size are optional.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
        // SAFETY: openpty returned two fresh descriptors that we now own.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        // Children of other tests must not keep the terminal open.
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            // SAFETY: fcntl on a descriptor we own.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        // SAFETY: fcntl on a descriptor we own.
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
        shell
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            shell.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = shell.spawn().expect("start the login shell");
        drop(shell);

        let output = Arc::new(Mutex::new(String::new()));
        let hang_up = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = std::thread::spawn({
            let output = Arc::clone(&output);
            let hang_up = Arc::clone(&hang_up);
            let mut master = std::fs::File::from(master);
            move || {
                let mut buf = [0; 4096];
                while !hang_up.load(Ordering::Relaxed) {
                    match master.read(&mut buf) {
                        Ok(n) if n > 0 => output
                            .lock()
                            .unwrap()
                            .push_str(&String::from_utf8_lossy(&buf[..n])),
                        _ => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
                // Closing the last descriptor of the master hangs up.
            }
        });
        Self {
            pgid: child.id(),
            shell: Some(child),
            output,
            hang_up,
            reader: Some(reader),
        }
    }

    /// Wait for the terminal to show what `pick` finds.
    fn wait_output<T>(&self, mut pick: impl FnMut(&str) -> Option<T>) -> T {
        for _ in 0..200 {
            if let Some(found) = pick(&self.output.lock().unwrap()) {
                return found;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("not on the terminal: {}", self.output.lock().unwrap());
    }

    /// Close the terminal, then SIGHUP the login's process group, as sshd
    /// does when the connection drops.
    fn hang_up(&mut self) {
        let Some(reader) = self.reader.take() else {
            return;
        };
        self.hang_up.store(true, Ordering::Relaxed);
        reader.join().unwrap();
        // SAFETY: killpg has no memory-safety preconditions.
        unsafe { libc::killpg(self.pgid as libc::pid_t, libc::SIGHUP) };
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.hang_up();
    }
}

/// Stops a server this test is not the parent of.
struct StopOnDrop(u32);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        // SAFETY: kill has no memory-safety preconditions.
        unsafe {
            libc::kill(self.0 as libc::pid_t, libc::SIGTERM);
        }
    }
}

/// Connect as the client instance `client_id` and say hello.
async fn connect_as(server: &Server, client_id: &str) -> Client {
    let mut client = Client::connect_silent(server).await;
    client.client_id = client_id.to_owned();
    client.hello().await;
    client
}

/// The close frame the server ends the connection with, read past anything
/// still queued.
async fn close_frame(client: &mut Client) -> Option<(u16, String)> {
    let close = async {
        while let Some(Ok(message)) = client.ws.next().await {
            if let Message::Close(frame) = message {
                return frame.map(|frame| (u16::from(frame.code), frame.reason.to_string()));
            }
        }
        None
    };
    tokio::time::timeout(TIMEOUT, close)
        .await
        .expect("connection left open")
}

#[tokio::test]
async fn a_client_back_on_a_new_connection_supersedes_its_old_one_at_once() {
    let server = Server::start(&[]);
    let mut stays = Client::connect(&server).await;
    let pane_id = stays.open_pane().await;
    // Two tabs of one app on this machine: client ids of their own.
    let mut old = connect_as(&server, "6c1d0c8e-0000-4000-8000-00000000000a").await;
    old.subscribe(pane_id).await;
    let mut tab = connect_as(&server, "6c1d0c8e-0000-4000-8000-00000000000b").await;
    tab.subscribe(pane_id).await;
    eventually_viewers(&mut stays, 3).await;
    let panes = stays.list_panes().await;
    let mut ids: Vec<&str> = panes[0]
        .viewed_by
        .iter()
        .map(|viewer| viewer.client_id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "",
            "6c1d0c8e-0000-4000-8000-00000000000a",
            "6c1d0c8e-0000-4000-8000-00000000000b"
        ]
    );
    assert!(
        panes[0]
            .viewed_by
            .iter()
            .all(|viewer| viewer.address.starts_with("127.0.0.1:")),
        "{:?}",
        panes[0].viewed_by
    );

    // The first client comes back on a new connection while the old one is
    // still open (its network left it half-open): the old one goes before the
    // new one hears its HelloAck, with no wait for the client timeout.
    let mut new = connect_as(&server, "6c1d0c8e-0000-4000-8000-00000000000a").await;
    assert_eq!(new.list_panes().await[0].viewers, 2);
    assert_eq!(
        close_frame(&mut old).await,
        Some((tilr_proto::close::SUPERSEDED, "superseded".into()))
    );
    server
        .wait_logged("client superseded by a newer connection")
        .await;
    // The new connection starts afresh: it subscribes as any other.
    new.subscribe(pane_id).await;
    assert_eq!(new.run(pane_id, "true").await, 0);
    assert_eq!(tab.run(pane_id, "true").await, 0);
    eventually_viewers(&mut stays, 3).await;
}

#[tokio::test]
async fn the_same_client_id_under_another_token_supersedes_nobody() {
    let id = "6c1d0c8e-0000-4000-8000-00000000000c";
    // A wrong token is turned away before it could take anyone's place.
    let server = Server::start(&[]);
    let mut first = connect_as(&server, id).await;
    let pane_id = first.open_pane().await;
    let mut spoof = Client::connect_silent(&server).await;
    spoof.client_id = id.to_owned();
    spoof.token = "not the token".into();
    spoof.hello_as(PROTOCOL_VERSION).await;
    assert_eq!(spoof.error().await, "wrong access token");
    assert_eq!(first.run(pane_id, "true").await, 0);
    assert_eq!(first.list_panes().await[0].viewers, 1);

    // A server without a token lets any in: another token is another user's
    // client, whatever id it gives.
    let server = Server::start(&["--no-auth"]);
    let mut first = Client::connect_silent(&server).await;
    first.client_id = id.to_owned();
    first.token = "one".into();
    first.hello().await;
    let pane_id = first.open_pane().await;
    let mut other = Client::connect_silent(&server).await;
    other.client_id = id.to_owned();
    other.token = "two".into();
    other.hello().await;
    other.subscribe(pane_id).await;
    assert_eq!(other.run(pane_id, "true").await, 0);
    assert_eq!(first.run(pane_id, "true").await, 0);
    assert_eq!(first.list_panes().await[0].viewers, 2);
    assert!(!server.logged("superseded"));
}

#[tokio::test]
async fn a_lagging_client_gets_the_last_screen_without_new_output() {
    // `slow` reads nothing while the output drains, so it answers no ping:
    // where that takes longer than the client timeout (30 s; a Dagger
    // container takes about that for 30 MB) the server rightly drops it as
    // silent. Lag, not liveness, is what this is about.
    let server = Server::start(&["--client-timeout", "0"]);
    let mut fast = Client::connect(&server).await;
    let pane_id = fast.open_pane_with(300, 100, "").await;
    // A client on a slow network: a small receive buffer fills at once.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let addr = server.addr.trim_start_matches("ws://").parse().unwrap();
    let stream = socket.connect(addr).await.unwrap();
    let (ws, _) = tokio_tungstenite::client_async(&server.addr, MaybeTlsStream::Plain(stream))
        .await
        .unwrap();
    let mut slow = Client::connect_silent(&server).await;
    slow.ws = ws;
    slow.hello().await;
    slow.subscribe(pane_id).await;
    while !next_frame(&mut slow).await.full {}

    // Heavy output `slow` does not read, then the pane goes quiet.
    fast.type_line(
        pane_id,
        "head -c 30000000 /dev/urandom | base64; echo \"<do\"\"ne>\"",
    )
    .await;
    fast.wait_screen("<done>").await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // The pane's last screen, whole: what `fast` has read so far can stop
    // short of it (the prompt after `<done>` comes in a frame of its own).
    fast.subscribe(pane_id).await;
    while !next_frame(&mut fast).await.full {}
    let last = fast.screen.text();

    // `slow` reads again: with nothing more printed, the screen it ends up
    // with is the pane's last one, brought whole after the lag.
    let mut full_after_lag = false;
    let caught_up = async {
        while slow.screen.text() != last {
            full_after_lag |= next_frame(&mut slow).await.full;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), caught_up)
        .await
        .unwrap_or_else(|_| panic!("stale screen:\n{}", slow.screen.text()));
    assert!(full_after_lag, "no full frame after the lag");
}

/// The next frame the pane sends.
async fn next_frame(client: &mut Client) -> Frame {
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) => Some(frame.clone()),
            _ => None,
        })
        .await
}
