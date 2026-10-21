//! Use cases: measuring the terminal pipeline. A client names its input
//! (`ClientMessage.seq`, from its `HelloAck.seq_base`) and finds the frame
//! that shows it by `Frame.last_input_seq`, with the pane's timing in
//! `Frame.timing`; the server's own counters are at `GET /metrics`, behind
//! the access token (or, on a server without one, for this machine only).

mod common;

use common::*;
use tilr_proto::MouseInput;

/// The seqs of one connection: `seq_base` to `seq_base + 2^32`.
fn own(seq_base: u64, seq: u64) -> bool {
    (seq_base..seq_base + (1 << 32)).contains(&seq)
}

/// Connect and say hello: the client and its `seq_base`.
async fn connect_with_base(server: &Server) -> (Client, u64) {
    let mut client = Client::connect_silent(server).await;
    let ack = client.hello().await;
    assert!(ack.seq_base >= 1 << 32, "{ack:?}");
    (client, ack.seq_base)
}

/// Read frames until one carries `seq`; every one before it must carry
/// something else allowed by `before`.
async fn frame_with_seq(client: &mut Client, seq: u64, before: impl Fn(u64) -> bool) -> Frame {
    loop {
        let frame = next_frame(client).await;
        if frame.last_input_seq == seq {
            return frame;
        }
        assert!(
            before(frame.last_input_seq),
            "frame with seq {} while waiting for {seq}",
            frame.last_input_seq
        );
    }
}

/// Whether no frame comes for `wait`.
async fn no_frame_within(client: &mut Client, wait: Duration) -> bool {
    tokio::time::timeout(wait, next_frame(client))
        .await
        .is_err()
}

/// Read frames until the pane has been quiet for a moment.
async fn settle(client: &mut Client) {
    while !no_frame_within(client, Duration::from_millis(300)).await {}
}

/// `line` and a newline, typed into the pane as input `seq`.
fn raw(pane_id: PaneId, line: &str) -> Msg {
    Msg::Raw(RawInput {
        pane_id: pane_id.to_string(),
        data: Bytes::from(format!("{line}\n")),
    })
}

/// Send `msg` named by `seq`.
async fn send_with_seq(client: &mut Client, msg: Msg, seq: u64) {
    let packet = tilr_proto::encode(&ClientMessage::with_seq(msg, seq));
    client.ws.send(Message::Binary(packet)).await.expect("send");
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

#[tokio::test]
async fn a_frame_names_the_input_it_shows_and_how_it_was_made() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client.wait_screen("$").await;
    let before = client.frame.clone().unwrap();
    assert_eq!(before.last_input_seq, 0);
    assert!(before.timing.is_some());

    send_with_seq(
        &mut client,
        Msg::Raw(RawInput {
            pane_id: pane_id.to_string(),
            data: Bytes::from("echo \"seq\"\"-seven\"\n"),
        }),
        7,
    )
    .await;
    let mut output_seen = false;
    loop {
        let frame = next_frame(&mut client).await;
        let timing = frame.timing.expect("every frame has its timing");
        if frame.last_input_seq == 7 {
            assert!(timing.build_us > 0, "{timing:?}");
            if timing.output_bytes > 0 {
                assert!(timing.reads > 0, "{timing:?}");
                output_seen = true;
            }
        } else {
            assert_eq!(frame.last_input_seq, 0);
        }
        if client.screen.text().contains("seq-seven") {
            break;
        }
    }
    assert!(output_seen, "no frame with output after the input");

    // Input without a seq leaves the last one in place; a scroll is input
    // too, credited (and answered) when applied even if it moves nothing.
    // This client numbers from 1, ignoring its seq_base: alone on the
    // pane, that works.
    client.type_line(pane_id, "echo two").await;
    client.wait_screen("two\n").await;
    assert_eq!(client.frame.as_ref().unwrap().last_input_seq, 7);
    send_with_seq(
        &mut client,
        Msg::Scroll(Scroll {
            pane_id: pane_id.to_string(),
            delta: 0,
            to_bottom: true,
        }),
        8,
    )
    .await;
    let scrolled = frame_with_seq(&mut client, 8, |seq| seq == 7).await;
    assert!(scrolled.changed.is_empty(), "{scrolled:?}");
    send_with_seq(&mut client, raw(pane_id, "echo three"), 9).await;
    frame_with_seq(&mut client, 9, |seq| seq == 8).await;
    client.wait_screen("three\n").await;
    assert_eq!(client.frame.as_ref().unwrap().last_input_seq, 9);
}

#[tokio::test]
async fn input_that_changes_nothing_is_answered_with_an_empty_frame() {
    let server = Server::start(&[]);
    let (mut client, base) = connect_with_base(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);
    settle(&mut client).await;
    let shown = client.frame.clone().unwrap();
    // A click the shell does not track writes nothing and changes nothing.
    let click = || {
        Msg::Mouse(MouseInput {
            pane_id: pane_id.to_string(),
            x: 3,
            y: 3,
            ..Default::default()
        })
    };

    // Without a seq, no frame; with one, a frame that says so.
    client.send(click()).await;
    assert!(no_frame_within(&mut client, Duration::from_millis(300)).await);
    send_with_seq(&mut client, click(), base + 1).await;
    let ack = next_frame(&mut client).await;
    assert_eq!(ack.last_input_seq, base + 1);
    assert!(!ack.full);
    assert!(ack.changed.is_empty(), "{ack:?}");
    assert!(ack.styles.is_empty() && ack.images.is_empty() && ack.title.is_none());
    assert!(!ack.placements_changed);
    assert_eq!(ack.seq, shown.seq + 1);
    assert_eq!((ack.cols, ack.rows), (shown.cols, shown.rows));
    assert_eq!(ack.cursor, shown.cursor);
    assert_eq!(ack.viewport_top, shown.viewport_top);
    assert_eq!(
        (ack.mouse_tracking, ack.alt_screen),
        (shown.mouse_tracking, shown.alt_screen)
    );
    let timing = ack.timing.unwrap();
    assert_eq!((timing.output_bytes, timing.wait_us), (0, 0), "{timing:?}");

    // Empty raw input too.
    send_with_seq(
        &mut client,
        Msg::Raw(RawInput {
            pane_id: pane_id.to_string(),
            data: Bytes::new(),
        }),
        base + 2,
    )
    .await;
    let ack = next_frame(&mut client).await;
    assert_eq!(ack.last_input_seq, base + 2);
    assert!(ack.changed.is_empty(), "{ack:?}");
}

#[tokio::test]
async fn a_client_that_reconnects_tells_its_input_from_before() {
    let server = Server::start(&[]);
    let (mut first, first_base) = connect_with_base(&server).await;
    let pane_id = first.open_pane().await;
    first.wait_screen("$").await;
    for n in 1..=3 {
        send_with_seq(
            &mut first,
            raw(pane_id, &format!("echo a{n}")),
            first_base + n,
        )
        .await;
        frame_with_seq(&mut first, first_base + n, |_| true).await;
    }
    drop(first);

    // A page reload: a new connection, a new base, counting from 1 again.
    let (mut second, base) = connect_with_base(&server).await;
    assert!(base > first_base, "{base} after {first_base}");
    second.subscribe(pane_id).await;
    let redraw = next_frame(&mut second).await;
    assert!(redraw.full);
    // The input it shows is the old connection's, not this one's 3.
    assert_eq!(redraw.last_input_seq, first_base + 3);
    assert!(!own(base, redraw.last_input_seq));

    send_with_seq(&mut second, raw(pane_id, "echo b1"), base + 1).await;
    frame_with_seq(&mut second, base + 1, |seq| seq == first_base + 3).await;
    second.wait_screen("b1\n").await;
}

#[tokio::test]
async fn two_clients_on_one_pane_each_see_their_own_input() {
    let server = Server::start(&[]);
    let (mut a, base_a) = connect_with_base(&server).await;
    let (mut b, base_b) = connect_with_base(&server).await;
    assert_ne!(base_a, base_b);
    let pane_id = a.open_pane().await;
    a.wait_screen("$").await;
    b.subscribe(pane_id).await;
    b.wait_screen("$").await;

    let mut last = 0;
    for n in 1..=2 {
        for (who, seq) in [(0, base_a + n), (1, base_b + n)] {
            let sender = if who == 0 { &mut a } else { &mut b };
            send_with_seq(sender, raw(pane_id, &format!("echo {seq}")), seq).await;
            // Both see it, in order: nothing newer before it.
            for client in [&mut a, &mut b] {
                let previous = last;
                frame_with_seq(client, seq, |s| s == previous).await;
            }
            last = seq;
        }
    }
    assert!(own(base_a, base_a + 2) && !own(base_b, base_a + 2));
}

#[tokio::test]
async fn a_scroll_is_credited_while_key_input_waits_for_the_pty() {
    let server = Server::start(&[]);
    let (mut client, base) = connect_with_base(&server).await;
    let pane_id = client.open_pane().await;
    // Nothing reads the pty, and the terminal is in raw mode: input piles
    // up until the pty takes no more.
    client
        .type_line(pane_id, "stty raw -echo; echo \"rea\"\"dy\"; sleep 60")
        .await;
    client.wait_screen("ready").await;
    send_with_seq(
        &mut client,
        Msg::Raw(RawInput {
            pane_id: pane_id.to_string(),
            data: Bytes::from(vec![b'x'; 1 << 20]),
        }),
        base + 1,
    )
    .await;
    send_with_seq(
        &mut client,
        Msg::Scroll(Scroll {
            pane_id: pane_id.to_string(),
            delta: 0,
            to_bottom: true,
        }),
        base + 2,
    )
    .await;
    // The raw input never got through, the scroll did.
    frame_with_seq(&mut client, base + 2, |seq| seq == 0).await;
}

#[tokio::test]
async fn the_last_input_and_input_still_waiting_survive_a_reload() {
    let server = Server::start(&[]);
    let (mut client, base) = connect_with_base(&server).await;
    let pane_id = client.open_pane().await;
    let (fifo, mut release) = activity_fifo(&server);
    send_with_seq(
        &mut client,
        raw(
            pane_id,
            &format!(
                "stty raw -echo; echo \"rea\"\"dy\"; read _ < '{}'; cat > /dev/null",
                fifo.display()
            ),
        ),
        base + 1,
    )
    .await;
    client.wait_screen("ready").await;
    assert_eq!(client.frame.as_ref().unwrap().last_input_seq, base + 1);
    // Waits for the pty until the shell is let go, the reload in between.
    send_with_seq(
        &mut client,
        Msg::Raw(RawInput {
            pane_id: pane_id.to_string(),
            data: Bytes::from(vec![b'x'; 1 << 20]),
        }),
        base + 2,
    )
    .await;
    // Passed to the pane process before the reload: the reload lets it take
    // what is queued for it before it goes.
    wait_sample_above(
        &server,
        &format!("tilrd_pane_input_total{{pane=\"{pane_id}\"}}"),
        1.0,
    )
    .await;

    // The pane process holds the input across the reload: it has waited at
    // least from here, before the reload began, to its release below.
    let held = std::time::Instant::now();
    let mut client = reload(&server, client).await;
    let (mut other, other_base) = connect_with_base(&server).await;
    assert!(other_base > base, "ids go on after a reload");
    let _ = other.ws.close(None).await;
    client.subscribe(pane_id).await;
    let redraw = next_frame(&mut client).await;
    assert!(redraw.full);
    assert_eq!(redraw.last_input_seq, base + 1);
    release_activity_fifo(&mut release);
    let held = held.elapsed();
    let through = frame_with_seq(&mut client, base + 2, |seq| seq == base + 1).await;
    // Counted from when the server got it, before `held` started; a clock
    // that restarted with the successor would count less than `held`.
    let waited = through.timing.unwrap().input_us;
    assert!(
        u128::from(waited) >= held.as_micros(),
        "{waited} µs, held {held:?} since the reload"
    );
}

#[tokio::test]
async fn metrics_ask_for_the_access_token() {
    let server = Server::start(&[]);
    let token = server.token();

    let (status, _) = http_get(&server, "/metrics", &[]).await;
    assert_eq!(status, 401);
    let (status, _) = http_get(&server, "/metrics?token=wrong", &[]).await;
    assert_eq!(status, 401);
    let (status, _) = http_get(&server, "/metrics", &[("Authorization", "Bearer wrong")]).await;
    assert_eq!(status, 401);
    let bearer = format!("Bearer {token}");
    let (status, _) = http_get(
        &server,
        "/metrics",
        &[
            ("Authorization", &bearer),
            ("Origin", "https://evil.example"),
        ],
    )
    .await;
    assert_eq!(status, 403);

    let (status, body) = http_get(&server, "/metrics", &[("Authorization", &bearer)]).await;
    assert_eq!(status, 200);
    assert_eq!(sample(&body, "tilrd_panes"), Some(0.0), "{body}");
    let (status, body) = http_get(&server, &format!("/metrics?token={token}"), &[]).await;
    assert_eq!(status, 200);
    assert!(body.contains("# TYPE tilrd_panes gauge"), "{body}");

    // The main protocol is where it was.
    let mut client = Client::connect(&server).await;
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn metrics_need_no_token_on_a_server_without_one() {
    let server = Server::start(&["--no-auth"]);
    let (status, body) = http_get(&server, "/metrics", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(sample(&body, "tilrd_panes"), Some(0.0), "{body}");
    let port = server.addr.rsplit(':').next().unwrap();
    for host in [
        "localhost".to_owned(),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
        "127.0.0.1".to_owned(),
    ] {
        let (status, _) = http_get(&server, "/metrics", &[("Host", &host)]).await;
        assert_eq!(status, 200, "{host}");
    }
    for site in ["same-origin", "none"] {
        let (status, _) = http_get(&server, "/metrics", &[("Sec-Fetch-Site", site)]).await;
        assert_eq!(status, 200, "{site}");
    }
}

#[tokio::test]
async fn without_a_token_metrics_are_for_this_machine_only() {
    let server = Server::start(&["--no-auth"]);
    // A page of another site, by fetch or <img>.
    let (status, _) = http_get(&server, "/metrics", &[("Sec-Fetch-Site", "cross-site")]).await;
    assert_eq!(status, 403);
    // A page that rebinds its own name to this machine.
    let port = server.addr.rsplit(':').next().unwrap();
    for host in [
        "evil.example".to_owned(),
        format!("evil.example:{port}"),
        format!("localhost.evil.example:{port}"),
    ] {
        let (status, _) = http_get(&server, "/metrics", &[("Host", &host)]).await;
        assert_eq!(status, 403, "{host}");
    }
    let (status, _) = http_request(&server, &["GET /metrics HTTP/1.1\r\n\r\n"]).await;
    assert_eq!(status, 403, "no Host");

    // With a token, the token is what counts.
    let server = Server::start(&[]);
    let url = format!("/metrics?token={}", server.token());
    let headers = [("Host", "evil.example"), ("Sec-Fetch-Site", "cross-site")];
    let (status, _) = http_get(&server, &url, &headers).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_metrics_request_may_come_in_pieces() {
    let server = Server::start(&[]);
    let token = server.token();
    let addr = server.addr.trim_start_matches("ws://");
    let rest = format!("rics?token={token} HTTP/1.1\r\nHost: {addr}\r");
    let (status, body) = http_request(&server, &["GET /met", &rest, "\n\r", "\n"]).await;
    assert_eq!(status, 200);
    assert!(body.contains("# TYPE tilrd_panes gauge"), "{body}");
}

#[tokio::test]
async fn metrics_count_frames_input_and_routing() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client.type_line(pane_id, "echo \"rea\"\"dy\"").await;
    client.wait_screen("ready").await;
    let pid = client.list_panes().await[0].pid;

    let url = format!("/metrics?token={}", server.token());
    let (status, text) = http_get(&server, &url, &[]).await;
    assert_eq!(status, 200);
    let pane = format!("{{pane=\"{pane_id}\"}}");
    let at_least = |series: &str, min: f64| {
        let value = sample(&text, series).unwrap_or_else(|| panic!("no {series}:\n{text}"));
        assert!(value >= min, "{series} = {value}:\n{text}");
    };
    at_least("tilrd_panes", 1.0);
    at_least("tilrd_clients", 1.0);
    at_least(&format!("tilrd_pane_frames_total{pane}"), 1.0);
    at_least(&format!("tilrd_pane_frame_bytes_total{pane}"), 1.0);
    at_least(&format!("tilrd_pane_frames_sent_total{pane}"), 1.0);
    at_least(&format!("tilrd_pane_input_total{pane}"), 1.0);
    at_least(&format!("tilrd_pane_input_bytes_total{pane}"), 1.0);
    at_least("tilrd_route_seconds_count", 1.0);
    at_least("tilrd_route_input_seconds_count", 1.0);
    at_least("tilrd_supervisor_loop_lag_seconds_count", 1.0);
    at_least("tilrd_route_seconds_bucket{le=\"+Inf\"}", 1.0);
    let supervisor = server.child.id();
    at_least(
        &format!("tilrd_process_memory_bytes{{role=\"supervisor\",pid=\"{supervisor}\"}}"),
        1.0,
    );
    at_least(
        &format!("tilrd_process_memory_bytes{{role=\"pane\",pid=\"{pid}\",pane=\"{pane_id}\"}}"),
        1.0,
    );
    at_least(
        &format!(
            "tilrd_process_cpu_seconds_total{{role=\"pane\",pid=\"{pid}\",pane=\"{pane_id}\"}}"
        ),
        0.0,
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("tilrd_info{version=\"")),
        "{text}"
    );
    assert!(
        text.lines().all(|line| {
            !line.starts_with("tilr_")
                && !line.starts_with("# HELP tilr_")
                && !line.starts_with("# TYPE tilr_")
        }),
        "old metric prefix in:\n{text}"
    );
}

#[tokio::test]
async fn metrics_count_the_clients_let_go_and_why() {
    let server = Server::start(&[]);
    let url = format!("/metrics?token={}", server.token());
    let drops = |text: &str, reason: &str| {
        sample(
            text,
            &format!("tilrd_client_drops_total{{reason=\"{reason}\"}}"),
        )
    };
    let (_, text) = http_get(&server, &url, &[]).await;
    assert_eq!(drops(&text, "superseded"), Some(0.0), "{text}");

    let mut refused = Client::connect_silent(&server).await;
    refused.token = "wrong token".into();
    refused.hello_as(PROTOCOL_VERSION).await;
    assert!(matches!(
        refused.next().await,
        Some(server_message::Msg::Error(_))
    ));
    assert!(refused.next().await.is_none());
    let (_, text) = http_get(&server, &url, &[]).await;
    for reason in ["superseded", "too_slow", "silent", "no_pong", "no_hello"] {
        assert_eq!(drops(&text, reason), Some(0.0), "{text}");
    }

    let mut voluntary = Client::connect(&server).await;
    voluntary.ws.close(None).await.expect("close voluntarily");
    server.wait_logged("client disconnected").await;
    let (_, text) = http_get(&server, &url, &[]).await;
    for reason in ["superseded", "too_slow", "silent", "no_pong", "no_hello"] {
        assert_eq!(drops(&text, reason), Some(0.0), "{text}");
    }

    let mut old = Client::connect_silent(&server).await;
    old.client_id = "6c1d0c8e-0000-4000-8000-00000000000d".into();
    old.hello().await;
    let mut new = Client::connect_silent(&server).await;
    new.client_id = old.client_id.clone();
    new.hello().await;
    while old.next().await.is_some() {}
    let (_, text) = http_get(&server, &url, &[]).await;
    assert_eq!(drops(&text, "superseded"), Some(1.0), "{text}");
    assert_eq!(drops(&text, "too_slow"), Some(0.0), "{text}");
}

#[tokio::test]
async fn a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo() {
    let server = Server::start(&[]);
    let (mut fast, fast_base) = connect_with_base(&server).await;
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
    let slow_base = slow.hello().await.seq_base;
    slow.subscribe(pane_id).await;
    // Frames of the shell's prompt may still be on their way ahead of it.
    while !next_frame(&mut slow).await.full {}

    // Output far faster than `slow` reads, which is not at all for a while:
    // its frames pile up behind the socket.
    fast.type_line(
        pane_id,
        "head -c 30000000 /dev/urandom | base64; sleep 1; echo \"<do\"\"ne>\"",
    )
    .await;
    // `slow` lags once the server skips frames for it.
    wait_sample_above(
        &server,
        &format!("tilrd_pane_frames_skipped_total{{pane=\"{pane_id}\"}}"),
        0.0,
    )
    .await;
    // Input of both while `slow` lags: its own comes first, so the frames
    // that echo it are among those it misses. A frame carries only the
    // latest input credited, so `fast` sends once it has seen a frame that
    // echoes `slow`'s: the server has passed that frame by `slow` (skipped,
    // its seq kept for the full frame that ends the lag) before `fast`'s
    // input can be credited in one with it.
    send_with_seq(&mut slow, raw(pane_id, ""), slow_base + 1).await;
    while next_frame(&mut fast).await.last_input_seq != slow_base + 1 {}
    send_with_seq(&mut fast, raw(pane_id, ""), fast_base + 1).await;
    // `fast` keeps reading meanwhile, up to the echo of its input: the pane
    // has taken `slow`'s input before it, and nothing is credited after it.
    while next_frame(&mut fast).await.last_input_seq != fast_base + 1 {}

    // `slow` reads again: frames it was behind on, then a whole screen, the
    // echo of its input, and the end of the output like everyone.
    let mut echoed = false;
    let mut full_after_lag = false;
    let mut frames = 0;
    let caught_up = async {
        while !echoed || !slow.screen.text().contains("<done>") {
            let frame = next_frame(&mut slow).await;
            frames += 1;
            full_after_lag |= frame.full;
            echoed |= frame.last_input_seq == slow_base + 1;
        }
    };
    // `fast` answers the server's pings meanwhile, as a client does.
    let reading = async {
        loop {
            fast.next().await.expect("connection closed");
        }
    };
    tokio::select! {
        () = caught_up => {}
        () = reading => unreachable!(),
    }
    assert!(
        full_after_lag,
        "no full frame after the lag ({frames} frames)"
    );
    fast.wait_screen("<done>").await;

    let url = format!("/metrics?token={}", server.token());
    let (_, text) = http_get(&server, &url, &[]).await;
    let skipped = sample(
        &text,
        &format!("tilrd_pane_frames_skipped_total{{pane=\"{pane_id}\"}}"),
    )
    .unwrap_or_default();
    let produced = sample(
        &text,
        &format!("tilrd_pane_frames_total{{pane=\"{pane_id}\"}}"),
    )
    .unwrap_or_default();
    assert!(skipped > 0.0, "nothing skipped:\n{text}");
    assert!(
        (frames as f64) < produced,
        "{frames} frames of {produced} went to the lagging client"
    );
    assert_eq!(
        sample(&text, "tilrd_client_drops_total{reason=\"too_slow\"}"),
        Some(0.0)
    );
}
