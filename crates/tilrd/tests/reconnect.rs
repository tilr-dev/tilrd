//! Use cases: a client goes away and comes back.

mod common;

use common::*;

/// Walk each reconnect boundary under explicit controller release. The FIFO
/// makes output happen only after Unsubscribe has been acknowledged. The ready
/// marker has no trailing bytes that could produce another frame after it is seen.
#[cfg(feature = "failpoints")]
async fn controlled_reconnect() -> (Server, Client, PaneId, PauseController) {
    let server = Server::start(&["--client-timeout", "0"]);
    let controller = PauseController::bind(&server.dir);
    let mut first = Client::connect(&server).await;
    let (pane, mut startup) = pane_before_shell_prompt(&server, &mut first, 300, 100).await;
    let (fifo, mut release) = activity_fifo(&server, "activity-release");
    let command = script(
        &server,
        "wait-for-output.sh",
        &format!(
            "printf '\\nready for release'; read -r release < '{}'; printf '\\nprinted while unwatched\\n'\n",
            fifo.display()
        ),
    );
    first.type_line(pane, &command).await;
    first.wait_screen("wait-for-output.sh").await;
    release_activity_fifo(&mut startup);
    first.wait_row("ready for release").await;
    first
        .send(Msg::Unsubscribe(Unsubscribe {
            pane_id: pane.to_string(),
        }))
        .await;
    assert_eq!(first.list_panes().await[0].viewers, 0);
    drop(first);

    let points = server.dir.join("failpoints");
    let output = format!("reconnect-unwatched-output:{pane}");
    std::fs::write(&points, format!("{output}=pause")).unwrap();
    release_activity_fifo(&mut release);
    let output_hit = controller.wait(&output).await;
    std::fs::write(&points, "reconnect-client-accepted=pause").unwrap();
    output_hit.release().await;

    // Bound the kernel buffer so the lag variant exercises an unread socket.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let addr = server.addr.trim_start_matches("ws://").parse().unwrap();
    let stream = socket.connect(addr).await.unwrap();
    let (ws, _) = tokio_tungstenite::client_async(&server.addr, MaybeTlsStream::Plain(stream))
        .await
        .unwrap();
    let mut client = Client::from_socket(&server, ws);
    let accepted = controller.wait("reconnect-client-accepted").await;
    let subscribe = format!("reconnect-subscribe:{pane}");
    let full = format!("reconnect-full-frame:{pane}");
    std::fs::write(&points, format!("{subscribe}=pause;{full}=pause")).unwrap();
    accepted.release().await;
    client.hello().await;
    client.subscribe(pane).await;
    let subscribed = controller.wait(&subscribe).await;
    std::fs::write(&points, format!("{full}=pause")).unwrap();
    subscribed.release().await;
    let sending_full = controller.wait(&full).await;
    std::fs::write(&points, "").unwrap();
    sending_full.release().await;
    assert!(next_frame(&mut client).await.full);
    assert!(
        client.screen.has_row("printed while unwatched"),
        "first reconnect frame missed unwatched output:\n{}",
        client.screen.text()
    );
    (server, client, pane, controller)
}

/// The startup file stops the real /bin/sh before its initial prompt. An
/// observer watches that boundary while the normal opening helper is polled.
#[tokio::test]
async fn opening_a_fresh_pane_waits_for_the_first_shell_prompt() {
    let dir = Server::scratch_dir();
    let startup = dir.join("held-shell-startup");
    let server = Server::start_with_env(dir, &[], &[("ENV", startup.to_str().unwrap())]);
    let (fifo, mut release) = activity_fifo(&server, "shell-startup-release");
    std::fs::write(
        &startup,
        format!(
            "set +o emacs; set +o vi\nprintf 'shell startup held\\n'\nread -r release < '{}'\n",
            fifo.display()
        ),
    )
    .unwrap();
    let mut observer = Client::connect(&server).await;
    let mut client = Client::connect(&server).await;
    let pane = {
        let opening = client.open_pane();
        tokio::pin!(opening);
        let held = async {
            let pane: PaneId = observer.pushed_pane_list().await[0]
                .pane_id
                .parse()
                .unwrap();
            observer.subscribe(pane).await;
            let mut subscribed = false;
            let mut startup_held = false;
            while !subscribed || !startup_held {
                if let Some(server_message::Msg::PaneList(list)) = observer.next().await {
                    subscribed |= list
                        .panes
                        .iter()
                        .any(|info| info.pane_id == pane.to_string() && info.viewers == 2);
                }
                startup_held |= observer.screen.text().contains("shell startup held");
            }
        };
        tokio::select! {
            biased;
            _ = &mut opening => panic!("opening returned before the first shell prompt"),
            _ = held => {}
        }
        // Two viewers prove that opening passed PaneCreated and sent Subscribe.
        // Pending now means waiting for the prompt, not for pane creation.
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(opening.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        release_activity_fifo(&mut release);
        opening.await
    };
    // Split the literal so PTY echo cannot satisfy the output wait. No leading
    // newline hides a prompt prefix: the opening helper must prevent that race.
    client.type_line(pane, "echo first out''put").await;
    client.wait_row("first output").await;
}

/// Ordered frames make an old pane's prompt arrive before the wanted one.
#[tokio::test]
async fn shell_readiness_ignores_prompts_from_other_panes() {
    let server = Server::start(&[]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let wanted: PaneId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let other: PaneId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    let frames = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        for pane in [other, wanted] {
            let msg = ServerMessage::from(server_message::Msg::Frame(Frame {
                pane_id: pane.to_string(),
                full: true,
                cols: 100,
                rows: 30,
                changed: vec![tilr_proto::Row {
                    y: 0,
                    runs: vec![tilr_proto::Run {
                        text: "$ ".into(),
                        ..Default::default()
                    }],
                }],
                ..Default::default()
            }));
            ws.send(Message::Binary(tilr_proto::encode(&msg)))
                .await
                .unwrap();
        }
    });
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .unwrap();
    let mut client = Client::from_socket(&server, ws);
    client.wait_shell_prompt(wanted).await;
    assert_eq!(client.frame.as_ref().unwrap().pane_id, wanted.to_string());
    frames.await.unwrap();
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_test_controls_output_accept_subscribe_and_full_frame_on_reconnect() {
    let (_server, mut client, pane, _controller) = controlled_reconnect().await;
    assert_eq!(client.run(pane, "true").await, 0);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_test_pauses_socket_reads_until_reconnected_client_lags() {
    let (server, mut client, pane, controller) = controlled_reconnect().await;
    let hook = format!("reconnect-client-lagging:{pane}");
    let points = server.dir.join("failpoints");
    std::fs::write(&points, format!("{hook}=pause")).unwrap();
    let stop = server.dir.join("stop-output");
    let command = script(
        &server,
        "produce-until-lag.sh",
        &format!(
            "while [ ! -e '{}' ]; do head -c 65536 /dev/urandom | base64; done\nprintf '\\nlag output stopped\\n'\n",
            stop.display()
        ),
    );
    let reads = client.pause_reads();
    client.type_line(pane, &command).await;
    // Keep a read in flight while output fills the socket. Without the pause,
    // next consumes a message and this branch fails before lag is observed.
    let lagging = tokio::select! {
        biased;
        message = client.next() => panic!("socket read completed while paused: {message:?}"),
        hit = controller.wait(&hook) => hit,
    };
    // Stop at the observed decision, rather than guessing how many bytes fill
    // a particular machine's buffers. The shell acknowledges the stop on screen.
    std::fs::write(&stop, "stop").unwrap();
    std::fs::write(&points, "").unwrap();
    reads.resume();
    lagging.release().await;
    let mut recovered_full = false;
    while !client.screen.has_row("lag output stopped") {
        recovered_full |= next_frame(&mut client).await.full;
    }
    assert!(recovered_full, "lag recovery did not send a full frame");
    assert_eq!(client.run(pane, "true").await, 0);
}

#[tokio::test]
#[should_panic(expected = "read pause dropped without resume")]
async fn a_dropped_read_pause_fails_the_next_read() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let reads = client.pause_reads();
    drop(reads);
    client.next().await;
}

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
async fn reconnect_output_when_input_precedes_the_shell_prompt() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let (pane, mut release) = pane_before_shell_prompt(&server, &mut client, 100, 30).await;
    // A leading newline isolates the output from a prompt printed after echo.
    client
        .type_line(pane, "printf '\\nprinted la''ter\\n'")
        .await;
    client.wait_screen("printf").await;
    client
        .send(Msg::Unsubscribe(Unsubscribe {
            pane_id: pane.to_string(),
        }))
        .await;
    assert_eq!(client.list_panes().await[0].viewers, 0);
    drop(client);
    release_activity_fifo(&mut release);
    let mut client = Client::connect(&server).await;
    client.subscribe(pane).await;
    client.wait_screen("printed later").await;
    assert!(
        client.screen.has_row("printed later"),
        "screen:\n{}",
        client.screen.text()
    );
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn output_printed_while_nobody_watched_is_there_on_reconnect() {
    let (_server, client, _pane, _controller) = controlled_reconnect().await;
    assert!(client.screen.has_row("printed while unwatched"));
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
    // takes to answer the server's pings. A window, not a bound on a wait:
    // a client wrongly taken for silent is dropped one timeout in, so three
    // leave room for a late check.
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

    // Rules out a timeout applied though it is off: the shortest one, 1 s,
    // taken for 0 would have dropped the silent client within 2 s.
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
    let (mut server, mut login, pid) = login_server("");
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
    // The SIGHUPs of the hang-up were sent by the time the login shell is
    // gone. One that ended a process by its default action would have done
    // so before the process runs again, so the server, the pane process and
    // the shell each answering below shows that none of them was ended so.
    let mut client = Client::connect(&server).await;
    client.subscribe(pane_id).await;
    client.type_line(pane_id, "echo \"kept=$KEPT\"").await;
    client.wait_row("kept=yes").await;
    // The server's own SIGHUP handler runs on a task of its own, unordered
    // with the requests above, and what it logs went to the terminal that
    // hung up. The window rules out that handler ending the server: by the
    // answer above the server has run since the signal and its handler
    // task only waits to be scheduled, which a second covers; a server
    // shutting down would then refuse the command below.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    assert!(process_alive(pid), "the server is still running");
    assert!(process_alive(pane_pid), "the pane process is still running");
    assert!(process_alive(shell_pid), "the shell is still running");

    // Stopping it still ends the shells.
    // SAFETY: kill has no memory-safety preconditions.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    eventually("the shell exits", || !process_alive(shell_pid)).await;
    eventually("the server exits", || !process_alive(pid)).await;
}

/// Start the same terminal/session topology for both hang-up tests.
fn login_server(before_start: &str) -> (Server, Login, u32) {
    // `tilrd &` typed into an SSH login; the dir goes to `Server`,
    // which removes it.
    let dir = Server::scratch_dir();
    let mut shell = Command::new("/bin/sh");
    shell
        .arg("-c")
        .arg(format!(
            "{before_start}\"$0\" --listen 127.0.0.1:0 & echo \"server pid $!\"; wait"
        ))
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
    let server = Server {
        owner,
        child: login.shell.take().unwrap(),
        addr,
        dir,
        log: Arc::default(),
        log_changed: Arc::default(),
    };
    (server, login, pid)
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_test_holds_the_login_shell_after_hangup_and_releases_its_exit() {
    let (mut server, mut login, pid) = login_server(HangupExitPause::TRAP);
    let _stop = StopOnDrop(pid);
    let mut client = Client::connect(&server).await;
    let pane = client.open_pane().await;
    client.type_line(pane, "KEPT=controlled").await;
    assert_eq!(client.run(pane, "true").await, 0);
    drop(client);

    let mut held = HangupExitPause::new(server.child.id());
    login.hang_up();
    held.wait_stopped().await;
    assert!(server.is_running(), "login shell is held after SIGHUP");
    let mut client = Client::connect(&server).await;
    assert_eq!(client.list_panes().await[0].pane_id, pane.to_string());
    client.subscribe(pane).await;
    assert_eq!(client.run(pane, "true").await, 0);
    drop(client);

    let status = held.release_and_wait(&mut server.child).await;
    assert_eq!(status.code(), Some(129));
    let mut client = Client::connect(&server).await;
    client.subscribe(pane).await;
    client.type_line(pane, "echo kept=$KEPT").await;
    client.wait_row("kept=controlled").await;
}

/// An SSH login: a shell leading a session of its own, with a pty as its
/// controlling terminal. What the terminal shows is collected until it hangs
/// up.
struct Login {
    shell: Option<Child>,
    /// The login shell's pid, which names its session and process group.
    pgid: u32,
    output: Arc<(Mutex<String>, std::sync::Condvar)>,
    hang_up: std::os::unix::net::UnixStream,
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

        let output = Arc::new((Mutex::new(String::new()), std::sync::Condvar::new()));
        let (hang_up, wake) = std::os::unix::net::UnixStream::pair().unwrap();
        let reader = std::thread::spawn({
            let output = Arc::clone(&output);
            let mut master = std::fs::File::from(master);
            move || {
                let mut buf = [0; 4096];
                loop {
                    let mut fds = [
                        libc::pollfd {
                            fd: master.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: wake.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    // SAFETY: both descriptors and the pollfd array remain valid.
                    let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
                    if ready < 0 {
                        assert_eq!(
                            std::io::Error::last_os_error().kind(),
                            std::io::ErrorKind::Interrupted
                        );
                        continue;
                    }
                    if fds[1].revents != 0 {
                        break;
                    }
                    match master.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            output
                                .0
                                .lock()
                                .unwrap()
                                .push_str(&String::from_utf8_lossy(&buf[..n]));
                            output.1.notify_all();
                        }
                        _ => break,
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
        let deadline = std::time::Instant::now() + TIMEOUT;
        let mut output = self.output.0.lock().unwrap();
        loop {
            if let Some(found) = pick(&output) {
                return found;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(!remaining.is_zero(), "not on the terminal: {output}");
            output = self.output.1.wait_timeout(output, remaining).unwrap().0;
        }
    }

    /// Close the terminal, then SIGHUP the login's process group, as sshd
    /// does when the connection drops.
    fn hang_up(&mut self) {
        let Some(reader) = self.reader.take() else {
            return;
        };
        use std::io::Write;
        self.hang_up.write_all(b"h").unwrap();
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
    // `read` keeps the shell from printing its prompt after `<done>`: the
    // screen with `<done>` is the pane's last.
    fast.type_line(
        pane_id,
        "head -c 30000000 /dev/urandom | base64; echo \"<do\"\"ne>\"; read _",
    )
    .await;
    fast.wait_screen("<done>").await;
    // The pane's last screen, whole.
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
