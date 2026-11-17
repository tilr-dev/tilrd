//! Use cases: the terminal on the bus (`pane.*` of the participant `tilrd`):
//! panes created, listed and closed by calls, their events, watching a pane as a
//! stream of frames, a lagging watcher, input acknowledged once applied and
//! applied once across reconnects, reading a pane as text. `tilr.v1` clients
//! see the same panes and frames meanwhile.
//!
//! Each change appends its tests as one block at the end of this file.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use bytes::Bytes;
use common::bus::{BusOptions, BusServer};
use common::bus_panes::{
    Screen, Sink, Watching, call, changed_until, create, create_shell, list, next_reply, raw,
    raw_with_client_id, read, send_call, send_call_bytes, shell, wait_text,
};
use common::bus_streams::within;
use common::{Client, TIMEOUT};
use prost::Message;
use tilr_bus::bus::{Call, Credit, envelope::Msg, reply};
use tilr_bus::pane::{
    AppliedReply, AppliedRequest, CloseReply, CloseRequest, CreateRequest, InputAck,
    InputFinalizeReply, InputFinalizeRequest, InputRequest, KeyAction, KeyInput, ListRequest,
    MouseAction, MouseButton, MouseInput, NamedKey, OutputRequest, PaneActivity, PaneActivityKind,
    PaneExited, PaneInfo, Paste, RawInput, ReadReply, ReadRequest, ResizeRequest, Scroll,
    input_request::Input,
};
use tilr_bus::{Bus, BusError, CallOptions, ConnectOptions, codes};

// ---------------------------------------------------------------------------
// bus-core-panes
// ---------------------------------------------------------------------------

fn start() -> BusServer {
    BusServer::start(BusOptions::default())
}

fn sh(script: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

/// `pane.input {raw}` with an `input_seq`.
async fn numbered(
    bus: &Bus,
    pane_id: &str,
    input_seq: u64,
    data: &str,
) -> Result<InputAck, BusError> {
    call(
        bus,
        "pane.input",
        &InputRequest {
            pane_id: pane_id.into(),
            input: Some(Input::Raw(RawInput {
                data: Bytes::copy_from_slice(data.as_bytes()),
            })),
            input_seq,
            ..Default::default()
        },
    )
    .await
}

async fn applied(bus: &Bus) -> Vec<(String, u64)> {
    call::<_, AppliedReply>(bus, "pane.applied", &AppliedRequest {})
        .await
        .expect("pane.applied")
        .applied
        .into_iter()
        .map(|ack| (ack.pane_id, ack.seq))
        .collect()
}

#[tokio::test]
async fn a_pane_is_created_listed_and_closed_by_calls() {
    let server = start();
    let owner = server.owner().await;
    assert!(list(&owner).await.is_empty());

    let info = create_shell(&owner).await;
    assert!(
        info.pane_id.parse::<uuid::Uuid>().is_ok(),
        "{}",
        info.pane_id
    );
    assert_eq!((info.cols, info.rows), (100, 30));
    assert_eq!(info.command, shell());
    assert_eq!(info.created_by, "owner");
    let panes = list(&owner).await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, info.pane_id);
    assert_eq!((panes[0].cols, panes[0].rows), (100, 30));
    assert_eq!(panes[0].viewers, 0);

    // A tilr.v1 client sees the same pane.
    let mut client = Client::connect(&server).await;
    client.hello().await;
    let listed = client.list_panes().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].pane_id, info.pane_id);

    // A pane id that names no pane, and the connection stays.
    let error = call::<_, CloseReply>(
        &owner,
        "pane.close",
        &CloseRequest {
            pane_id: "1".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (codes::NOT_FOUND, "no such pane: 1")
    );
    assert_eq!(list(&owner).await.len(), 1);

    // Closed: answered once the program ended.
    let exited = create(
        &owner,
        80,
        24,
        sh("trap 'exit 7' HUP; echo trapped; while :; do sleep 1; done"),
    )
    .await;
    // The trap is set once it says so.
    wait_text(&owner, &exited.pane_id, |lines| {
        lines.first().is_some_and(|line| line == "trapped")
    })
    .await;
    let reply: CloseReply = call(
        &owner,
        "pane.close",
        &CloseRequest {
            pane_id: exited.pane_id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(reply.exit_code, 7);
    let _: CloseReply = call(
        &owner,
        "pane.close",
        &CloseRequest {
            pane_id: info.pane_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(list(&owner).await.is_empty());
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn pane_changes_are_events() {
    let server = start();
    let owner = server.owner().await;
    let subscriber = server.owner().await;
    let mut events = subscriber.subscribe("pane.*").await.unwrap();

    // Each step waits for the test's Enter, so the events come in a known order.
    let info = create(
        &owner,
        80,
        24,
        sh("stty -echo; printf '\\033]2;t\\007'; read a; printf '\\a'; read b; exit 3"),
    )
    .await;
    let scope = format!("pane:{}", info.pane_id);
    let mut seen = Vec::new();
    let mut next = async |seen: &mut Vec<String>, until: &str| loop {
        let event = tokio::time::timeout(TIMEOUT, events.next())
            .await
            .unwrap_or_else(|_| panic!("no {until} in time; saw {seen:?}"))
            .expect("the subscription ended");
        assert_eq!(event.scope, scope, "{}", event.subject);
        assert_eq!(event.from, "tilrd");
        seen.push(event.subject.clone());
        if event.subject == until {
            return event;
        }
    };

    let created = next(&mut seen, "pane.created").await;
    assert_eq!(
        PaneInfo::decode(created.payload).unwrap().pane_id,
        info.pane_id
    );
    let title = next(&mut seen, "pane.title").await;
    let title = PaneInfo::decode(title.payload).unwrap();
    assert_eq!(
        (title.pane_id.as_str(), title.title.as_str()),
        (info.pane_id.as_str(), "t")
    );
    raw(&owner, &info.pane_id, "\n").await.unwrap();
    let bell = PaneActivity::decode(next(&mut seen, "pane.bell").await.payload).unwrap();
    assert_eq!(bell.kind, PaneActivityKind::Bell as i32);
    assert!(bell.at_ms > 0);
    raw(&owner, &info.pane_id, "\n").await.unwrap();
    let exited = PaneExited::decode(next(&mut seen, "pane.exited").await.payload).unwrap();
    assert_eq!(exited.exit_code, 3);

    let order: Vec<&str> = seen
        .iter()
        .map(String::as_str)
        .filter(|subject| *subject != "pane.changed")
        .collect();
    assert_eq!(
        order,
        ["pane.created", "pane.title", "pane.bell", "pane.exited"]
    );
    // The title changed the pane's info too.
    assert!(
        seen.iter().any(|subject| subject == "pane.changed"),
        "{seen:?}"
    );
}

#[tokio::test]
async fn watching_a_pane_is_a_stream_of_frames() {
    let server = start();
    let owner = server.owner_with(ConnectOptions::new("watcher-test")).await;
    let info = create_shell(&owner).await;
    let pane_id: uuid::Uuid = info.pane_id.parse().unwrap();
    // A tilr.v1 subscriber of the same pane.
    let mut client = Client::connect(&server).await;
    client.hello().await;
    client.subscribe(pane_id).await;

    let mut watching = Watching::open(&owner, &info.pane_id).await.unwrap();
    let first = watching.next_frame().await.unwrap().unwrap();
    assert!(first.full, "the first chunk is a full frame");
    assert_eq!(first.pane_id, info.pane_id);
    assert_eq!(watching.screen.rows[0], "$");

    raw(&owner, &info.pane_id, "echo \"sum is $((6 * 7))\"\n")
        .await
        .unwrap();
    watching.wait_screen("sum is 42").await;
    client.wait_screen("sum is 42").await;

    let panes = list(&owner).await;
    assert_eq!(panes[0].viewers, 2);
    let bus_viewer = panes[0]
        .viewed_by
        .iter()
        .find(|viewer| viewer.client_name == "watcher-test")
        .expect("the stream's caller is a viewer");
    assert_eq!(bus_viewer.identity, "owner");

    // Cancelled: the caller is no viewer any more.
    let mut changes = owner.subscribe("pane.changed").await.unwrap();
    drop(watching);
    changed_until(&mut changes, &info.pane_id, |info| {
        info.viewers == 1
            && !info
                .viewed_by
                .iter()
                .any(|viewer| viewer.client_name == "watcher-test")
    })
    .await;

    // The stream ends with End{PaneExited} when the program exits.
    let mut watching = Watching::open(&owner, &info.pane_id).await.unwrap();
    assert!(watching.next_frame().await.unwrap().unwrap().full);
    raw(&owner, &info.pane_id, "exit 5\n").await.unwrap();
    while let Some(frame) = watching.next_frame().await {
        frame.unwrap();
    }
    let end = watching.stream.end_payload().expect("End");
    let exited = PaneExited::decode(end.clone()).unwrap();
    assert_eq!(
        (exited.pane_id.as_str(), exited.exit_code),
        (info.pane_id.as_str(), 5)
    );

    // A pane that does not exist.
    let error = Watching::open(&owner, "1").await.unwrap();
    let mut error = error;
    let ended = error.next_frame().await;
    match ended {
        Some(Err(error)) => {
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                (codes::NOT_FOUND, "no such pane: 1")
            );
        }
        other => panic!("expected not_found, got {other:?}"),
    }
}

#[tokio::test]
async fn a_lagging_watcher_skips_to_the_latest_screen() {
    let server = start();
    let owner = server.owner().await;
    let info = create_shell(&owner).await;
    let mut laggard = server.raw_greeted("laggard", Vec::new()).await;
    open_output(&mut laggard, 1, &info.pane_id, LAGGARD_WINDOW).await;
    let mut screen = Screen::default();
    assert!(credited_chunk(&mut laggard, 1, &mut screen).await);

    // The laggard reads nothing while the pane prints some 7 MB.
    raw(&owner, &info.pane_id, "seq 1 1000000; echo fin''ished\n")
        .await
        .unwrap();
    wait_text(&owner, &info.pane_id, |lines| {
        let last = lines.iter().rposition(|line| !line.is_empty());
        last.is_some_and(|last| last > 0 && lines[last] == "$" && lines[last - 1] == "finished")
    })
    .await;
    let now = read(
        &owner,
        &ReadRequest {
            pane_id: info.pane_id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // It reads again: a full frame of the screen as it is now, and it was not
    // dropped for lagging.
    within(async {
        while screen.rows != now.lines {
            credited_chunk(&mut laggard, 1, &mut screen).await;
        }
    })
    .await;
    assert!(
        screen.full_frames >= 2,
        "the laggard caught up with a full frame ({} frames, {} full)",
        screen.frames,
        screen.full_frames
    );
    assert!(laggard.close.is_none());
    assert_eq!(list(&owner).await[0].viewers, 1);
}

/// The next chunk of the `pane.output` stream `id` on `raw`, applied to
/// `screen` and credited; whether it was a full frame.
async fn credited_chunk(raw: &mut common::bus::Raw, id: u64, screen: &mut Screen) -> bool {
    match raw.recv().await {
        Some(Msg::Chunk(chunk)) => {
            assert_eq!(chunk.id, id);
            let bytes = chunk.payload.len() as u32;
            let payload = tilr_bus::Chunk {
                payload: chunk.payload,
                content_encoding: chunk.content_encoding,
            }
            .bytes()
            .unwrap();
            let frame = screen.apply_chunk(&payload);
            raw.send(Msg::Credit(Credit { id, bytes })).await;
            frame.is_some_and(|frame| frame.full)
        }
        other => panic!("expected a chunk of pane.output, got {other:?}"),
    }
}

/// The window of a reader meant to fall behind: a full frame of a 100x30
/// screen of numbers fits, a few dozen frames of `seq` output do not.
const LAGGARD_WINDOW: u32 = 8 << 10;

/// Open `pane.output` of `pane_id` as call `id` of `raw` with a `window`.
async fn open_output(raw: &mut common::bus::Raw, id: u64, pane_id: &str, window: u32) {
    raw.send(Msg::Call(Call {
        id,
        method: "pane.output".into(),
        payload: OutputRequest {
            pane_id: pane_id.into(),
            ..Default::default()
        }
        .encode_to_vec()
        .into(),
        window,
        ..Call::default()
    }))
    .await;
}

#[tokio::test]
async fn input_is_acknowledged_after_it_is_applied() {
    let server = start();
    let owner = server.owner().await;

    // A program that reads without echoing: the call is answered all the same.
    let quiet = create(&owner, 80, 24, sh("stty -echo; cat > /dev/null")).await;
    let ack = raw(&owner, &quiet.pane_id, "x").await.unwrap();
    assert_eq!((ack.pane_id.as_str(), ack.seq), (quiet.pane_id.as_str(), 0));

    // 100 inputs sent without waiting reach the program in that order.
    let mut out = Sink::new(server.dir.join("order.txt"));
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", out.path.display())),
    )
    .await;
    let mut raw_client = server.raw_greeted("orderly", Vec::new()).await;
    for n in 1..=100u64 {
        let request = InputRequest {
            pane_id: pane.pane_id.clone(),
            input: Some(Input::Raw(RawInput {
                data: format!("{n}\n").into(),
            })),
            ..Default::default()
        };
        raw_client
            .send(Msg::Call(Call {
                id: n,
                method: "pane.input".into(),
                payload: request.encode_to_vec().into(),
                ..Call::default()
            }))
            .await;
    }
    let mut answered = HashSet::new();
    while answered.len() < 100 {
        match raw_client.recv().await {
            Some(Msg::Reply(reply)) => {
                assert!(
                    matches!(reply.result, Some(reply::Result::Payload(_))),
                    "{reply:?}"
                );
                answered.insert(reply.id);
            }
            other => panic!("expected a reply, got {other:?}"),
        }
    }
    let expected: String = (1..=100).map(|n| format!("{n}\n")).collect();
    out.holds(&expected).await;

    // The seq is echoed in a frame, seq_base being connection_id << 32.
    assert_eq!(owner.seq_base(), owner.connection_id() << 32);
    let mut watching = Watching::open(&owner, &pane.pane_id).await.unwrap();
    assert!(watching.next_frame().await.unwrap().unwrap().full);
    let seq = owner.seq_base() + 1;
    let _: InputAck = call(
        &owner,
        "pane.input",
        &InputRequest {
            pane_id: pane.pane_id.clone(),
            input: Some(Input::Raw(RawInput {
                data: Bytes::from_static(b"z"),
            })),
            seq,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    loop {
        let frame = watching.next_frame().await.unwrap().unwrap();
        if frame.last_input_seq == seq {
            break;
        }
    }

    // Input to a pane that does not exist.
    let error = raw(&owner, "1", "x").await.unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (codes::NOT_FOUND, "no such pane: 1")
    );
}

#[tokio::test]
async fn numbered_input_is_applied_once_across_reconnects() {
    let server = start();
    let anonymous = server.owner().await;
    let mut out = Sink::new(server.dir.join("numbered.txt"));
    let pane = create(
        &anonymous,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", out.path.display())),
    )
    .await;
    let error = numbered(&anonymous, &pane.pane_id, 1, "0\n")
        .await
        .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            codes::BAD_REQUEST,
            "numbered input requires a stable client_id"
        )
    );

    let first = server
        .owner_with(ConnectOptions::new("numbered").client_id("numbered-1"))
        .await;
    let ack = numbered(&first, &pane.pane_id, 1, "1\n").await.unwrap();
    assert_eq!(ack.seq, 1);
    assert_eq!(applied(&first).await, [(pane.pane_id.clone(), 1)]);
    // A missing pane: not_found, and no counter moves.
    let missing = uuid::Uuid::new_v4().to_string();
    let error = numbered(&first, &missing, 2, "x\n").await.unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND);
    assert_eq!(applied(&first).await, [(pane.pane_id.clone(), 1)]);
    first.close();

    // Reconnected: the counters are there, a replay is not applied again.
    let again = server
        .owner_with(ConnectOptions::new("numbered").client_id("numbered-1"))
        .await;
    assert_eq!(applied(&again).await, [(pane.pane_id.clone(), 1)]);
    assert_eq!(
        numbered(&again, &pane.pane_id, 1, "1\n").await.unwrap().seq,
        1
    );
    assert_eq!(
        numbered(&again, &pane.pane_id, 2, "2\n").await.unwrap().seq,
        2
    );
    out.holds("1\n2\n").await;
    // Another client's counters are its own: none, as it sent nothing.
    let other = server
        .owner_with(ConnectOptions::new("numbered").client_id("numbered-2"))
        .await;
    assert!(applied(&other).await.is_empty());

    // Finalized: no counters, numbered input refused, plain input still taken.
    let _: InputFinalizeReply = call(&again, "pane.input_finalize", &InputFinalizeRequest {})
        .await
        .unwrap();
    assert!(applied(&again).await.is_empty());
    let error = numbered(&again, &pane.pane_id, 3, "3\n").await.unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST);
    raw(&again, &pane.pane_id, "4\n").await.unwrap();
    out.holds("1\n2\n4\n").await;
}

#[tokio::test]
async fn reading_a_pane_and_waiting_for_a_change() {
    let server = start();
    let owner = server.owner().await;
    let typist = server.owner().await;
    let info = create_shell(&owner).await;
    let read_now = async |request: ReadRequest| read(&owner, &request).await.unwrap();

    let first = read_now(ReadRequest {
        pane_id: info.pane_id.clone(),
        ..Default::default()
    })
    .await;
    assert!(first.alive);
    assert_eq!(first.exit_code, None);
    assert_eq!((first.cols, first.rows), (100, 30));
    assert_eq!(first.lines.len(), 30);
    assert_eq!(first.lines[0], "$");
    let cursor = first.cursor.unwrap();
    assert!(cursor.visible);
    assert_eq!((cursor.x, cursor.y), (2, 0));
    assert!(first.history.is_empty());

    // Waiting for a change: answered when a client types, about 1 s later. The
    // shell may still draw after its prompt; wait until its screen holds still.
    let first = within(async {
        let mut first = first;
        loop {
            let next = read_now(ReadRequest {
                pane_id: info.pane_id.clone(),
                since_seq: Some(first.seq),
                wait_ms: Some(300),
                ..Default::default()
            })
            .await;
            let still = next.seq == first.seq;
            first = next;
            if still {
                break first;
            }
        }
    })
    .await;
    // The read waits once its call is in: a call sent after it on the same
    // connection is answered first (calls of a connection run in order).
    let mut waiting = server.raw_greeted("waiting-reader", Vec::new()).await;
    let request = ReadRequest {
        pane_id: info.pane_id.clone(),
        since_seq: Some(first.seq),
        wait_ms: Some(5000),
        ..Default::default()
    };
    send_call(&mut waiting, 1, "pane.read", &request).await;
    send_call(&mut waiting, 2, "pane.list", &ListRequest {}).await;
    let (id, listed) = next_reply(&mut waiting).await;
    assert_eq!(id, 2, "the read did not wait: {listed:?}");
    raw(&typist, &info.pane_id, "echo typed\n").await.unwrap();
    let (id, changed) = next_reply(&mut waiting).await;
    assert_eq!(id, 1);
    let changed = ReadReply::decode(changed.unwrap()).unwrap();
    assert!(changed.seq > first.seq);
    // It shows the typing: it came after it.
    assert!(changed.lines[0].starts_with("$ e"), "{:?}", changed.lines);

    // Nothing changes: the same seq once the wait is over. The shell may still
    // draw after its prompt: a read that saw a change goes on from it.
    let mut quiet = wait_text(&owner, &info.pane_id, |lines| {
        lines.get(1).is_some_and(|line| line == "typed")
            && lines.get(2).is_some_and(|line| line == "$")
    })
    .await;
    within(async {
        loop {
            let started = std::time::Instant::now();
            let unchanged = read_now(ReadRequest {
                pane_id: info.pane_id.clone(),
                since_seq: Some(quiet.seq),
                wait_ms: Some(300),
                ..Default::default()
            })
            .await;
            let waited = started.elapsed();
            if unchanged.seq != quiet.seq {
                assert!(unchanged.seq > quiet.seq);
                quiet = unchanged;
                continue;
            }
            assert_eq!(unchanged.lines, quiet.lines);
            assert!(
                waited >= Duration::from_millis(280) && waited < Duration::from_secs(4),
                "{waited:?}"
            );
            break;
        }
    })
    .await;
    // A deadline shorter than the wait: answered before it, not `timeout`.
    let started = std::time::Instant::now();
    let unchanged: ReadReply = owner
        .call_typed(
            "pane.read",
            &ReadRequest {
                pane_id: info.pane_id.clone(),
                since_seq: Some(quiet.seq),
                wait_ms: Some(5000),
                ..Default::default()
            },
            CallOptions::default().timeout(Duration::from_millis(500)),
        )
        .await
        .unwrap();
    assert!(unchanged.seq >= quiet.seq);
    assert!(started.elapsed() < Duration::from_millis(500));

    // History above the viewport, oldest first, as much as asked for.
    raw(&owner, &info.pane_id, "seq 1 50\n").await.unwrap();
    wait_text(&owner, &info.pane_id, |lines| {
        let last = lines.iter().rposition(|line| !line.is_empty());
        last.is_some_and(|last| last > 0 && lines[last] == "$" && lines[last - 1] == "50")
    })
    .await;
    let all = read_now(ReadRequest {
        pane_id: info.pane_id.clone(),
        history_lines: 100_000,
        ..Default::default()
    })
    .await;
    assert!(all.history.len() >= 10, "{:?}", all.history);
    assert_eq!(all.history[0], "$ echo typed");
    assert_eq!(all.lines[29], "$");
    assert_eq!(all.lines[28], "50");
    let some = read_now(ReadRequest {
        pane_id: info.pane_id.clone(),
        history_lines: 10,
        ..Default::default()
    })
    .await;
    assert_eq!(some.history, all.history[all.history.len() - 10..]);
    assert_eq!(some.lines, all.lines);

    // A pane that does not exist.
    let error = read(
        &owner,
        &ReadRequest {
            pane_id: "1".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND);
}

/// `pane.input` of `input` without seqs.
async fn input(bus: &Bus, pane_id: &str, input: Input) -> InputAck {
    call(
        bus,
        "pane.input",
        &InputRequest {
            pane_id: pane_id.into(),
            input: Some(input),
            ..Default::default()
        },
    )
    .await
    .expect("pane.input")
}

fn key(named: NamedKey, text: &str) -> Input {
    Input::Key(KeyInput {
        action: KeyAction::Press as i32,
        named: named as i32,
        codepoint: text.chars().next().map_or(0, u32::from),
        text: text.into(),
        ..Default::default()
    })
}

#[tokio::test]
async fn a_pane_is_resized_by_a_call() {
    let server = start();
    let owner = server.owner().await;
    let info = create_shell(&owner).await;
    let ack: InputAck = call(
        &owner,
        "pane.resize",
        &ResizeRequest {
            pane_id: info.pane_id.clone(),
            cols: 60,
            rows: 20,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ack.pane_id, info.pane_id);
    // Answered once applied: the list and the screen have the new size.
    let listed = list(&owner).await;
    assert_eq!((listed[0].cols, listed[0].rows), (60, 20));
    let now = read(
        &owner,
        &ReadRequest {
            pane_id: info.pane_id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!((now.cols, now.rows, now.lines.len()), (60, 20, 20));
    // The program sees it.
    raw(&owner, &info.pane_id, "stty size\n").await.unwrap();
    wait_text(&owner, &info.pane_id, |lines| {
        lines.iter().any(|line| line == "20 60")
    })
    .await;
}

#[tokio::test]
async fn keys_mouse_paste_and_scroll_reach_the_pane() {
    let server = start();
    let owner = server.owner().await;

    // A key, a paste and Enter, as the program reads them.
    let mut typed = Sink::new(server.dir.join("typed.txt"));
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", typed.path.display())),
    )
    .await;
    input(&owner, &pane.pane_id, key(NamedKey::Unspecified, "k")).await;
    input(
        &owner,
        &pane.pane_id,
        Input::Paste(Paste {
            text: "pasted".into(),
        }),
    )
    .await;
    input(&owner, &pane.pane_id, key(NamedKey::Enter, "")).await;
    typed.holds("kpasted\n").await;

    // A mouse press, once the program asked for SGR mouse reports.
    let mut clicked = Sink::new(server.dir.join("clicked.txt"));
    let mouse = create(
        &owner,
        80,
        24,
        sh(&format!(
            "stty -echo raw; printf '\\033[?1000h\\033[?1006hready'; head -c 9 > '{}'",
            clicked.path.display()
        )),
    )
    .await;
    wait_text(&owner, &mouse.pane_id, |lines| lines[0] == "ready").await;
    input(
        &owner,
        &mouse.pane_id,
        Input::Mouse(MouseInput {
            action: MouseAction::Press as i32,
            button: MouseButton::Left as i32,
            x: 3,
            y: 2,
            ..Default::default()
        }),
    )
    .await;
    clicked.holds("\x1b[<0;4;3M").await;

    // Scrolling moves the viewport into history; answered once it did.
    let shell = create_shell(&owner).await;
    raw(&owner, &shell.pane_id, "seq 1 50\n").await.unwrap();
    let bottom = wait_text(&owner, &shell.pane_id, |lines| {
        lines[28] == "50" && lines[29] == "$"
    })
    .await;
    let scroll = |delta, to_bottom| Input::Scroll(Scroll { delta, to_bottom });
    input(&owner, &shell.pane_id, scroll(-5, false)).await;
    let read_now = async || {
        read(
            &owner,
            &ReadRequest {
                pane_id: shell.pane_id.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    };
    let back = read_now().await;
    assert_eq!(back.lines[5..], bottom.lines[..25]);
    input(&owner, &shell.pane_id, scroll(0, true)).await;
    assert_eq!(read_now().await.lines, bottom.lines);
}

#[tokio::test]
async fn a_replayed_input_still_pending_is_written_once() {
    let server = start();
    let owner = server.owner().await;
    let fifo = server.dir.join("go");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    let payload = server.dir.join("payload");
    let mut tail = Sink::new(server.dir.join("tail"));
    // The reader waits on the fifo: the input stays pending for the pty.
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!(
            "stty -echo -icanon min 1 time 0; printf 'reader-%s\\n' ready; read -r _ < '{}'; \
             head -c 1048576 > '{}'; printf 'payload-%s\\n' saved; head -c 4 > '{}'",
            fifo.display(),
            payload.display(),
            tail.path.display()
        )),
    )
    .await;
    wait_text(&owner, &pane.pane_id, |lines| lines[0] == "reader-ready").await;

    let mut client = raw_with_client_id(&server, "replaying", "replaying-1").await;
    let big = InputRequest {
        pane_id: pane.pane_id.clone(),
        input: Some(Input::Raw(RawInput {
            data: vec![b'a'; 1 << 20].into(),
        })),
        input_seq: 1,
        ..Default::default()
    };
    send_call(&mut client, 1, "pane.input", &big).await;
    // The same input again while its bytes wait for the pty.
    send_call(&mut client, 2, "pane.input", &big).await;
    send_call(&mut client, 3, "pane.applied", &AppliedRequest {}).await;
    // Neither is answered before the counters, which say nothing is applied.
    let (id, counters) = next_reply(&mut client).await;
    assert_eq!(id, 3, "an input was answered before it was applied");
    let counters = AppliedReply::decode(counters.unwrap()).unwrap();
    assert_eq!(
        counters.applied,
        [InputAck {
            pane_id: pane.pane_id.clone(),
            seq: 0
        }]
    );

    // The reader reads: the original's last byte goes in, both are answered.
    tokio::task::spawn_blocking(move || std::fs::write(fifo, "\n"))
        .await
        .unwrap()
        .unwrap();
    let mut answered = Vec::new();
    for _ in 0..2 {
        let (id, ack) = next_reply(&mut client).await;
        let ack = InputAck::decode(ack.unwrap()).unwrap();
        assert_eq!(ack.seq, 1);
        answered.push(id);
    }
    answered.sort_unstable();
    assert_eq!(answered, [1, 2]);
    // One copy: what comes next reaches the second reader whole.
    wait_text(&owner, &pane.pane_id, |lines| {
        lines.iter().any(|line| line == "payload-saved")
    })
    .await;
    let end = InputRequest {
        pane_id: pane.pane_id.clone(),
        input: Some(Input::Raw(RawInput {
            data: Bytes::from_static(b"END\n"),
        })),
        input_seq: 2,
        ..Default::default()
    };
    send_call(&mut client, 4, "pane.input", &end).await;
    assert_eq!(next_reply(&mut client).await.0, 4);
    tail.holds("END\n").await;
    assert_eq!(std::fs::read(&payload).unwrap(), vec![b'a'; 1 << 20]);
}

#[tokio::test]
async fn internal_fields_cannot_be_forged() {
    let server = start();
    let owner = server.owner().await;
    let mut out = Sink::new(server.dir.join("forged.txt"));
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", out.path.display())),
    )
    .await;
    let victim = server
        .owner_with(ConnectOptions::new("victim").client_id("victim-1"))
        .await;
    numbered(&victim, &pane.pane_id, 1, "v\n").await.unwrap();

    // `pane.input` with the fields `tilr.v1`'s ClientMessage uses towards pane
    // processes (102 input_identity, 103 input_probe, 104 input_forget): not
    // fields of InputRequest, so read as unknown and dropped.
    let mut forged = InputRequest {
        pane_id: pane.pane_id.clone(),
        input: Some(Input::Raw(RawInput {
            data: Bytes::from_static(b"x\n"),
        })),
        ..Default::default()
    }
    .encode_to_vec();
    forged.extend_from_slice(&[0xb2, 0x06, 6]);
    forged.extend_from_slice(b"forged");
    forged.extend_from_slice(&[0xb8, 0x06, 5, 0xc0, 0x06, 1]);
    let mut attacker = server.raw_greeted("attacker", Vec::new()).await;
    send_call_bytes(&mut attacker, 1, "pane.input", forged).await;
    let (_, ack) = next_reply(&mut attacker).await;
    assert_eq!(InputAck::decode(ack.unwrap()).unwrap().seq, 0);
    out.holds("v\nx\n").await;
    assert_eq!(applied(&victim).await, [(pane.pane_id.clone(), 1)]);

    // `created_by` (PaneInfo field 12) sent with pane.create is not taken.
    let mut create = CreateRequest {
        cols: 80,
        rows: 24,
        command: shell(),
        ..Default::default()
    }
    .encode_to_vec();
    create.extend_from_slice(&[0x62, 15]);
    create.extend_from_slice(b"instance:forged");
    send_call_bytes(&mut attacker, 2, "pane.create", create).await;
    let (_, info) = next_reply(&mut attacker).await;
    assert_eq!(PaneInfo::decode(info.unwrap()).unwrap().created_by, "owner");
}

#[tokio::test]
async fn a_read_the_pane_process_does_not_answer_ends() {
    let server = start();
    let owner = server.owner().await;
    let info = create_shell(&owner).await;
    let pane_process = info.pid as libc::pid_t;
    // SAFETY: kill has no memory-safety preconditions.
    assert_eq!(unsafe { libc::kill(pane_process, libc::SIGSTOP) }, 0);

    // No answer from the pane process: unavailable before the deadline, not a
    // call that hangs or times out.
    let request = ReadRequest {
        pane_id: info.pane_id.clone(),
        ..Default::default()
    };
    let error = owner
        .call_typed::<_, ReadReply>(
            "pane.read",
            &request,
            CallOptions::default().timeout(Duration::from_secs(1)),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::UNAVAILABLE, "{error:?}");

    // The pane ends while a read waits for its text: the read says so.
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    send_call(&mut reader, 1, "pane.read", &request).await;
    send_call(&mut reader, 2, "pane.list", &ListRequest {}).await;
    assert_eq!(next_reply(&mut reader).await.0, 2);
    // SAFETY: kill has no memory-safety preconditions.
    assert_eq!(unsafe { libc::kill(pane_process, libc::SIGKILL) }, 0);
    let (id, ended) = next_reply(&mut reader).await;
    assert_eq!(id, 1);
    let ended = ReadReply::decode(ended.unwrap()).unwrap();
    assert!(!ended.alive);
    assert!(ended.exit_code.is_some());
}

#[tokio::test]
async fn a_watcher_behind_when_the_pane_exits_gets_its_last_screen() {
    let server = start();
    let owner = server.owner().await;
    let subscriber = server.owner().await;
    let mut events = subscriber.subscribe("pane.exited").await.unwrap();
    // It prints some 7 MB once told to, then its last words, and exits.
    let info = create(
        &owner,
        100,
        30,
        sh("read go; seq 1 1000000; echo last''words; exit 4"),
    )
    .await;
    let mut laggard = server.raw_greeted("laggard", Vec::new()).await;
    open_output(&mut laggard, 1, &info.pane_id, LAGGARD_WINDOW).await;
    let mut screen = Screen::default();
    let mut chunk = async |laggard: &mut common::bus::Raw| match laggard.recv().await {
        Some(Msg::Chunk(chunk)) => {
            let bytes = chunk.payload.len() as u32;
            let payload = tilr_bus::Chunk {
                payload: chunk.payload,
                content_encoding: chunk.content_encoding,
            }
            .bytes()
            .unwrap();
            screen.apply_chunk(&payload);
            // Credit as read, as clients do: the last frame waits for it.
            laggard.send(Msg::Credit(Credit { id: 1, bytes })).await;
            None
        }
        Some(Msg::End(end)) => Some(PaneExited::decode(end.payload).unwrap()),
        other => panic!("expected pane.output, got {other:?}"),
    };
    assert!(chunk(&mut laggard).await.is_none());

    // The laggard reads nothing until the pane has exited.
    raw(&owner, &info.pane_id, "\n").await.unwrap();
    within(events.next()).await.expect("the subscription ended");
    let exited = loop {
        if let Some(exited) = chunk(&mut laggard).await {
            break exited;
        }
    };
    assert_eq!(exited.exit_code, 4);
    assert!(
        screen.rows.iter().any(|row| row == "lastwords"),
        "the last screen before End:\n{}",
        screen.text()
    );
}

#[tokio::test]
async fn a_slow_reader_of_pane_output_holds_up_no_other_pane_or_reader() {
    let server = start();
    let owner = server.owner().await;
    let mut changes = owner.subscribe("pane.changed").await.unwrap();
    let busy = create_shell(&owner).await;
    let other = create_shell(&owner).await;
    // A reader that stops reading its connection once it opened pane.output.
    let mut stalled = server.raw_greeted("stalled", Vec::new()).await;
    open_output(&mut stalled, 1, &busy.pane_id, LAGGARD_WINDOW).await;
    changed_until(&mut changes, &busy.pane_id, |info| info.viewers == 1).await;
    // Others watch both panes, on the bus and on tilr.v1.
    let watcher = server.owner().await;
    let mut watching_busy = Watching::open(&watcher, &busy.pane_id).await.unwrap();
    let mut watching_other = Watching::open(&watcher, &other.pane_id).await.unwrap();
    let mut client = Client::connect(&server).await;
    client.hello().await;
    client.subscribe(busy.pane_id.parse().unwrap()).await;

    raw(&owner, &busy.pane_id, "seq 1 1000000; echo busy''-done\n")
        .await
        .unwrap();
    // The other pane takes input and shows it while the busy one prints.
    raw(&owner, &other.pane_id, "echo other''-done\n")
        .await
        .unwrap();
    watching_other.wait_screen("other-done").await;
    // Every other reader of the busy pane sees it to the end.
    watching_busy.wait_screen("busy-done").await;
    client.wait_screen("busy-done").await;

    // The stalled reader reads again: the pane's screen as it is now, not a
    // backlog, and it is still connected.
    let now = wait_text(&owner, &busy.pane_id, |lines| {
        let last = lines.iter().rposition(|line| !line.is_empty());
        last.is_some_and(|last| last > 0 && lines[last] == "$" && lines[last - 1] == "busy-done")
    })
    .await;
    let mut screen = Screen::default();
    within(async {
        while screen.rows != now.lines {
            credited_chunk(&mut stalled, 1, &mut screen).await;
        }
    })
    .await;
    assert!(
        screen.full_frames >= 2,
        "it caught up with a full frame ({} frames, {} full)",
        screen.frames,
        screen.full_frames
    );
    assert!(stalled.close.is_none());
}

#[tokio::test]
async fn a_pane_call_past_its_deadline_is_answered_timeout_and_cancelled() {
    use common::bus_streams::{error_of, ping};
    let server = start();
    let owner = server.owner().await;
    let mut changes = owner.subscribe("pane.changed").await.unwrap();
    // A program that never reads its input, the pty not in line mode (which
    // drops what overflows): a megabyte of input is never applied.
    let deaf = create(
        &owner,
        80,
        24,
        sh("stty -echo -icanon min 1 time 0; echo de''af; exec sleep 100000"),
    )
    .await;
    wait_text(&owner, &deaf.pane_id, |lines| lines[0] == "deaf").await;
    let mut hasty = server.raw_greeted("hasty", Vec::new()).await;
    hasty
        .send(Msg::Call(Call {
            id: 1,
            method: "pane.input".into(),
            payload: InputRequest {
                pane_id: deaf.pane_id.clone(),
                input: Some(Input::Raw(RawInput {
                    data: vec![b'a'; 1 << 20].into(),
                })),
                ..Default::default()
            }
            .encode_to_vec()
            .into(),
            timeout_ms: 300,
            ..Call::default()
        }))
        .await;
    // A stream with a deadline: its caller is a viewer until then.
    hasty
        .send(Msg::Call(Call {
            id: 2,
            method: "pane.output".into(),
            payload: OutputRequest {
                pane_id: deaf.pane_id.clone(),
                ..Default::default()
            }
            .encode_to_vec()
            .into(),
            timeout_ms: 300,
            ..Call::default()
        }))
        .await;
    let mut timed_out = HashSet::new();
    while timed_out.len() < 2 {
        let msg = hasty.recv().await.expect("open");
        if matches!(msg, Msg::Chunk(_)) {
            continue;
        }
        let (id, code, message) = error_of(&msg).unwrap_or_else(|| panic!("{msg:?}"));
        assert_eq!(code, codes::TIMEOUT, "{id}: {message}");
        timed_out.insert(id);
    }
    // The callee side is stopped: the stream's caller is no viewer any more,
    // and the connection goes on.
    changed_until(&mut changes, &deaf.pane_id, |info| info.viewers == 1).await;
    changed_until(&mut changes, &deaf.pane_id, |info| info.viewers == 0).await;
    ping(&mut hasty, 3).await;
    assert!(list(&owner).await.iter().any(|p| p.pane_id == deaf.pane_id));
}

#[tokio::test]
async fn a_finished_command_is_a_pane_event() {
    let server = start();
    let owner = server.owner().await;
    let info = create_shell(&owner).await;
    let (fifo, mut release) = common::activity_fifo(&server, "finish-release");
    let command = common::script(
        &server,
        "foreground.sh",
        &format!(
            "printf 'foreground-ready\\n'\nread -r _ < '{}'\n",
            fifo.display()
        ),
    );
    raw(&owner, &info.pane_id, &format!("{command}\n"))
        .await
        .unwrap();
    wait_text(&owner, &info.pane_id, |lines| {
        lines.iter().any(|line| line == "foreground-ready")
    })
    .await;
    // Subscribed once the command runs in the foreground: what comes is its end.
    let mut events = owner.subscribe("pane.command_finished").await.unwrap();
    common::release_activity_fifo(&mut release);
    let event = within(events.next()).await.expect("the subscription ended");
    assert_eq!(event.subject, "pane.command_finished");
    assert_eq!(event.scope, format!("pane:{}", info.pane_id));
    assert_eq!(event.from, "tilrd");
    let finished = PaneActivity::decode(event.payload).unwrap();
    assert_eq!(finished.pane_id, info.pane_id);
    assert_eq!(finished.kind, PaneActivityKind::CommandFinished as i32);
    assert!(finished.at_ms > 0);
}

/// Kitty graphics: transmit image `id`, `side` x `side` pixels of noise (its
/// PNG about as large as its pixels), and place it at the cursor.
fn kitty_noise(id: u32, side: usize) -> Vec<u8> {
    use base64::Engine;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ u64::from(id);
    let pixels: Vec<u8> = (0..side * side * 3)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u8
        })
        .collect();
    let data = base64::engine::general_purpose::STANDARD.encode(&pixels);
    let parts: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let more = u8::from(i + 1 < parts.len());
        let head = if i == 0 {
            format!("\x1b_Ga=T,f=24,s={side},v={side},c=12,r=6,i={id},q=2,m={more};")
        } else {
            format!("\x1b_Gm={more};")
        };
        out.extend_from_slice(head.as_bytes());
        out.extend_from_slice(part);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

#[tokio::test]
async fn a_frame_over_the_window_goes_whole_to_a_reader_short_of_credit() {
    use tilr_bus::pane::{PaneOutput, pane_output::Output};
    const WINDOW: u32 = 64 << 10;
    let server = start();
    let owner = server.owner().await;
    // Three images of some 43 KiB each: two do not fit the window together.
    let images = server.dir.join("images");
    let shown: Vec<u8> = (1..=3).flat_map(|id| kitty_noise(id, 120)).collect();
    std::fs::write(&images, shown).unwrap();
    let info = create(
        &owner,
        100,
        30,
        sh(&format!(
            "stty -echo; cat '{}'; echo; echo sh''own; read x",
            images.display()
        )),
    )
    .await;
    wait_text(&owner, &info.pane_id, |lines| {
        lines.iter().any(|line| line == "shown")
    })
    .await;

    // Its first frame is full, with every image in view: more than the window.
    // The reader credits each chunk once it read it, so the second chunk waits.
    let mut reader = server.raw_greeted("short-of-credit", Vec::new()).await;
    open_output(&mut reader, 1, &info.pane_id, WINDOW).await;
    let mut screen = Screen::default();
    let mut run: Vec<(u32, u64)> = Vec::new();
    let frame = within(async {
        loop {
            let chunk = match reader.recv().await {
                Some(Msg::Chunk(chunk)) => chunk,
                other => panic!("images without their frame, then {other:?}; images {run:?}"),
            };
            assert_eq!(chunk.id, 1);
            let bytes = chunk.payload.len() as u32;
            assert!(bytes <= WINDOW, "a chunk of {bytes} bytes");
            let payload = tilr_bus::Chunk {
                payload: chunk.payload,
                content_encoding: chunk.content_encoding,
            }
            .bytes()
            .unwrap();
            let output = PaneOutput::decode(payload.as_ref()).unwrap();
            reader.send(Msg::Credit(Credit { id: 1, bytes })).await;
            match output.output {
                Some(Output::Images(images)) => {
                    for image in images.images {
                        let key = (image.id, image.generation);
                        assert!(!run.contains(&key), "image {key:?} twice: {run:?}");
                        run.push(key);
                    }
                }
                Some(Output::Frame(frame)) => {
                    screen.apply(&frame);
                    break frame;
                }
                None => panic!("an empty chunk"),
            }
        }
    })
    .await;
    assert!(frame.full);
    let mut placed: Vec<(u32, u64)> = frame
        .placements
        .iter()
        .map(|p| (p.image_id, p.image_generation))
        .collect();
    placed.sort_unstable();
    run.sort_unstable();
    assert_eq!(placed.iter().map(|p| p.0).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(run, placed, "the images the frame places, each once");
    assert!(screen.text().contains("shown"));
}

#[cfg(feature = "failpoints")]
mod failpoints {
    use super::*;
    use common::PauseController;

    /// What each pane call naming `pane_id` is answered: its error's code and
    /// message.
    async fn pane_errors(bus: &Bus, pane_id: &str) -> Vec<(&'static str, (String, String))> {
        let error = |method: &'static str, result: Result<(), BusError>| {
            let err = result.expect_err(method);
            (method, (err.code, err.message))
        };
        let closed = call::<_, CloseReply>(
            bus,
            "pane.close",
            &CloseRequest {
                pane_id: pane_id.into(),
            },
        )
        .await
        .map(drop);
        let typed = raw(bus, pane_id, "x").await.map(drop);
        let resized = call::<_, InputAck>(
            bus,
            "pane.resize",
            &ResizeRequest {
                pane_id: pane_id.into(),
                cols: 60,
                rows: 20,
                ..Default::default()
            },
        )
        .await
        .map(drop);
        let text = read(
            bus,
            &ReadRequest {
                pane_id: pane_id.into(),
                ..Default::default()
            },
        )
        .await
        .map(drop);
        let watched = match Watching::open(bus, pane_id).await {
            Err(err) => Err(err),
            Ok(mut watching) => match watching.next_frame().await {
                Some(Err(err)) => Err(err),
                other => panic!("pane.output of {pane_id}: {other:?}"),
            },
        };
        vec![
            error("pane.close", closed),
            error("pane.input", typed),
            error("pane.resize", resized),
            error("pane.read", text),
            error("pane.output", watched),
        ]
    }

    #[tokio::test]
    async fn a_pane_the_caller_may_not_access_is_answered_as_a_missing_one() {
        let server = start();
        let owner = server.owner().await;
        let info = create_shell(&owner).await;
        let missing = uuid::Uuid::new_v4().to_string();
        // The authorizer refuses every pane from here on.
        std::fs::write(server.dir.join("failpoints"), "bus-pane-access=error").unwrap();
        let refused = pane_errors(&owner, &info.pane_id).await;
        let absent = pane_errors(&owner, &missing).await;
        let answer = |pane_id: &str| {
            (
                codes::NOT_FOUND.to_owned(),
                format!("no such pane: {pane_id}"),
            )
        };
        for ((method, refused), (_, absent)) in refused.iter().zip(&absent) {
            assert_eq!(refused, &answer(&info.pane_id), "{method}");
            assert_eq!(absent, &answer(&missing), "{method}");
        }
        assert_eq!(refused.len(), 5);
        assert!(list(&owner).await.is_empty());
        // The pane ran all along.
        clear(&server);
        let listed: Vec<String> = list(&owner).await.into_iter().map(|p| p.pane_id).collect();
        assert_eq!(listed, [info.pane_id]);
    }

    fn pause(server: &BusServer, point: &str) {
        std::fs::write(server.dir.join("failpoints"), format!("{point}=pause")).unwrap();
    }

    fn clear(server: &BusServer) {
        std::fs::write(server.dir.join("failpoints"), "").unwrap();
    }

    #[tokio::test]
    async fn an_input_whose_reply_was_lost_is_applied_once() {
        let server = start();
        let pauses = PauseController::bind(&server.dir);
        let owner = server.owner().await;
        let mut out = Sink::new(server.dir.join("lost.txt"));
        let pane = create(
            &owner,
            80,
            24,
            sh(&format!("stty -echo; cat > '{}'", out.path.display())),
        )
        .await;
        let options = || ConnectOptions::new("lost").client_id("lost-1");
        let first = server.owner_with(options()).await;

        // Applied, and the connection ends before the acknowledgement.
        pause(&server, "input-before-ack");
        let sent = {
            let first = first.clone();
            let pane_id = pane.pane_id.clone();
            tokio::spawn(async move { numbered(&first, &pane_id, 1, "1\n").await })
        };
        let hit = pauses.wait("input-before-ack").await;
        first.close();
        assert_eq!(sent.await.unwrap().unwrap_err().code, codes::UNAVAILABLE);
        clear(&server);
        hit.release().await;

        // Reconnected: reported applied, and a replay is not applied again.
        let again = server.owner_with(options()).await;
        assert_eq!(applied(&again).await, [(pane.pane_id.clone(), 1)]);
        assert_eq!(
            numbered(&again, &pane.pane_id, 1, "1\n").await.unwrap().seq,
            1
        );
        assert_eq!(
            numbered(&again, &pane.pane_id, 2, "2\n").await.unwrap().seq,
            2
        );
        out.holds("1\n2\n").await;
    }

    #[tokio::test]
    async fn a_stuck_pane_is_closed_by_pane_applied() {
        let server = start();
        let pauses = PauseController::bind(&server.dir);
        let owner = server
            .owner_with(ConnectOptions::new("stuck").client_id("stuck-1"))
            .await;
        let cat = || sh("stty -echo; cat > /dev/null");
        let healthy = create(&owner, 80, 24, cat()).await;
        let stuck = create(&owner, 80, 24, cat()).await;
        let untouched = create(&owner, 80, 24, cat()).await;
        numbered(&owner, &healthy.pane_id, 1, "1\n").await.unwrap();
        numbered(&owner, &stuck.pane_id, 1, "1\n").await.unwrap();

        // The stuck pane does not answer the probe; the untouched one, which got
        // no numbered input, is not asked (it would stop too).
        std::fs::write(
            server.dir.join("failpoints"),
            format!(
                "input-before-probe:{}=pause;input-before-probe:{}=pause",
                stuck.pane_id, untouched.pane_id
            ),
        )
        .unwrap();
        let asking = {
            let owner = owner.clone();
            tokio::spawn(async move { applied(&owner).await })
        };
        let held = pauses
            .wait(&format!("input-before-probe:{}", stuck.pane_id))
            .await;
        // Within 5 s the stuck pane is closed and the others' counters come.
        assert_eq!(asking.await.unwrap(), [(healthy.pane_id.clone(), 1)]);
        clear(&server);
        drop(held); // Its pane process is gone; the controller must not release it.
        let listed: Vec<String> = list(&owner).await.into_iter().map(|p| p.pane_id).collect();
        assert!(!listed.contains(&stuck.pane_id), "{listed:?}");
        assert!(listed.contains(&healthy.pane_id) && listed.contains(&untouched.pane_id));
    }
}

// ---------------------------------------------------------------------------
// bus-core-services
// ---------------------------------------------------------------------------

mod services {
    use std::path::Path;

    use common::bus::TcpAuth;
    use common::bus_services::{
        self, Streamed, fifo_closed, methods, ran, ran_after, read_all, real, run, started,
        stream_all,
    };
    use tilr_bus::bus::{Kind, Lane};
    use tilr_bus::clipboard::{ClipboardData, GetRequest, PutReply, PutRequest};
    use tilr_bus::exec::{InputReply, InputRequest, RunRequest, SignalReply, SignalRequest};
    use tilr_bus::fs::{
        DeleteRequest, FileKind, FsAck, FsChange, FsChanged, FsData, FsEntries, FsInfo,
        ListRequest, MkdirRequest, ReadRequest, RenameRequest, StatRequest, UnwatchRequest,
        WatchReply, WatchRequest, WriteRequest,
    };
    use tilr_bus::pane::Copied;
    use tilr_bus::server::{CommandRequest, InfoReply, InfoRequest};
    use tilr_bus::{StreamOptions, Subscription};

    use super::*;

    const SERVICES: [&str; 18] = [
        "server.info",
        "server.reload",
        "server.update",
        "server.command",
        "fs.list",
        "fs.stat",
        "fs.read",
        "fs.write",
        "fs.mkdir",
        "fs.rename",
        "fs.delete",
        "fs.watch",
        "fs.unwatch",
        "exec.run",
        "exec.input",
        "exec.signal",
        "clipboard.put",
        "clipboard.get",
    ];

    fn path(path: &Path) -> String {
        path.to_str().unwrap().to_owned()
    }

    async fn next_event(events: &mut Subscription) -> tilr_bus::Event {
        within(events.next()).await.expect("the subscription ended")
    }

    #[tokio::test]
    async fn tilrd_provides_the_terminal_methods() {
        let server = start();
        let owner = server.owner().await;
        let listed = methods(&owner).await;
        let panes = [
            "pane.create",
            "pane.list",
            "pane.close",
            "pane.resize",
            "pane.input",
            "pane.read",
            "pane.applied",
            "pane.input_finalize",
            "pane.output",
        ];
        for method in panes.into_iter().chain(SERVICES) {
            let info = listed
                .iter()
                .find(|info| info.method == method)
                .unwrap_or_else(|| panic!("{method} is not listed"));
            assert_eq!(info.providers, ["tilrd"], "{method}");
        }
        let streams = [
            ("fs.read", Lane::Bulk),
            ("exec.run", Lane::Bulk),
            ("clipboard.get", Lane::Bulk),
            ("server.update", Lane::Bulk),
            ("server.command", Lane::Bulk),
        ];
        for (method, lane) in streams {
            let info = listed.iter().find(|info| info.method == method).unwrap();
            assert_eq!((info.kind(), info.lane()), (Kind::Stream, lane), "{method}");
        }
    }

    #[tokio::test]
    async fn server_info_describes_the_server() {
        let server = start();
        let owner = server.owner().await;
        let info: InfoReply = bus_services::call(&owner, "server.info", &InfoRequest {})
            .await
            .unwrap();
        // What tilr.v1's HelloAck says.
        let mut client = Client::connect(&server).await;
        let ack = client.hello().await;
        assert_eq!(info.server_version, ack.server_version);
        assert_eq!(info.server_id, ack.server_id);
        assert_eq!(info.server_id, owner.hello_ack().server_id);
        assert_eq!(info.update_channel, "stable");
        assert_eq!(
            info.features,
            ["clipboard", "files", "host-events", "agent", "exec"]
        );
        assert!(info.clipboard_max_bytes > 0 && info.fs_max_bytes > 0);
        assert_eq!(info.clipboard_max_bytes, ack.clipboard_max_bytes);
        assert_eq!(info.fs_max_bytes, ack.fs_max_bytes);
        // The same in JSON.
        let reply = within(owner.call(
            "server.info",
            "{}",
            CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
        ))
        .await
        .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&reply.payload).unwrap();
        assert_eq!(json["serverId"], info.server_id.as_str());
        assert_eq!(json["fsMaxBytes"], info.fs_max_bytes.to_string().as_str());

        // A feature turned off: not listed, its methods not provided.
        let server = BusServer::start(BusOptions {
            args: vec!["--no-exec".into(), "--no-files".into()],
            ..Default::default()
        });
        let owner = server.owner().await;
        let info: InfoReply = bus_services::call(&owner, "server.info", &InfoRequest {})
            .await
            .unwrap();
        assert_eq!(info.features, ["clipboard", "host-events", "agent"]);
        assert_eq!(info.fs_max_bytes, 0);
        for method in ["exec.run", "fs.list"] {
            let error = stream_all(&owner, method, &RunRequest::default(), 0)
                .await
                .end
                .unwrap_err();
            assert_eq!(error.code, codes::NOT_FOUND, "{method}: {error}");
        }
        // Server commands still run.
        let version = ran(stream_all(
            &owner,
            "server.command",
            &CommandRequest {
                argv: vec!["--version".into()],
                ..Default::default()
            },
            0,
        )
        .await);
        assert_eq!(version.exit.code, 0);
        assert!(!version.stdout.is_empty());
    }

    #[tokio::test]
    async fn files_are_listed_changed_and_watched_on_the_bus() {
        let server = start();
        let owner = server.owner().await;
        let dir = real(&server.dir, "shared");
        let mkdir = MkdirRequest {
            path: "~/shared".into(),
            ..Default::default()
        };
        bus_services::call::<_, FsAck>(&owner, "fs.mkdir", &mkdir)
            .await
            .unwrap();

        // Watched before anything is in it.
        let mut events = owner.subscribe("fs.changed.*").await.unwrap();
        let watch: WatchReply = bus_services::call(
            &owner,
            "fs.watch",
            &WatchRequest {
                path: path(&dir),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // A file written in two chunks: there only after the last.
        let file = dir.join("a.txt");
        let chunk = |data: &str, last: bool| WriteRequest {
            transfer_id: 1,
            path: path(&file),
            total_size: 10,
            create: true,
            data: Bytes::copy_from_slice(data.as_bytes()),
            last,
            ..Default::default()
        };
        let _: FsAck = bus_services::call(&owner, "fs.write", &chunk("hello", false))
            .await
            .unwrap();
        assert!(!file.exists(), "the file is replaced after the last chunk");
        let _: FsAck = bus_services::call(&owner, "fs.write", &chunk("world", true))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "helloworld");

        let changed: FsChanged = next_event(&mut events).await.decode().unwrap();
        assert_eq!(changed.watch_id, watch.watch_id);
        assert_eq!(
            (changed.path.as_str(), changed.kind()),
            (path(&file).as_str(), FsChange::Create)
        );

        let entries: FsEntries = bus_services::call(
            &owner,
            "fs.list",
            &ListRequest {
                path: "~/shared".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(entries.path, path(&dir));
        let names: Vec<_> = entries.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt"]);
        assert_eq!(entries.entries[0].size, 10);
        assert_eq!(entries.entries[0].kind(), FileKind::File);
        let info: FsInfo = bus_services::call(
            &owner,
            "fs.stat",
            &StatRequest {
                path: path(&file),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(info.entry.unwrap().size, 10);

        let rename = RenameRequest {
            from: path(&file),
            to: path(&dir.join("b.txt")),
            ..Default::default()
        };
        bus_services::call::<_, FsAck>(&owner, "fs.rename", &rename)
            .await
            .unwrap();
        assert!(dir.join("b.txt").exists() && !file.exists());
        let delete = DeleteRequest {
            path: path(&dir.join("b.txt")),
            ..Default::default()
        };
        bus_services::call::<_, FsAck>(&owner, "fs.delete", &delete)
            .await
            .unwrap();
        assert!(!dir.join("b.txt").exists());

        let unwatch = UnwatchRequest {
            watch_id: watch.watch_id,
        };
        bus_services::call::<_, FsAck>(&owner, "fs.unwatch", &unwatch)
            .await
            .unwrap();
        let error = bus_services::call::<_, FsAck>(&owner, "fs.unwatch", &unwatch)
            .await
            .unwrap_err();
        assert_eq!(error.code, codes::NOT_FOUND, "{error}");

        // Outside the shared directories: refused, the message for the user.
        let error = bus_services::call::<_, FsEntries>(
            &owner,
            "fs.list",
            &ListRequest {
                path: "/".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, codes::BAD_REQUEST);
        assert!(error.message.contains("outside the directories"), "{error}");

        // JSON for scripts.
        let reply = within(owner.call(
            "fs.stat",
            serde_json::json!({ "path": "~/shared" }).to_string(),
            CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
        ))
        .await
        .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&reply.payload).unwrap();
        assert_eq!(json["entry"]["kind"], "FILE_KIND_DIR");
        assert_eq!(json["path"], path(&dir).as_str());
    }

    #[tokio::test]
    async fn file_content_moves_in_chunks() {
        let server = start();
        let owner = server.owner().await;
        let file = real(&server.dir, "big");
        let content: Vec<u8> = (0..4u32 << 20).map(|i| (i % 251) as u8).collect();
        std::fs::write(&file, &content).unwrap();

        // With a 256 KiB window, every chunk fits it, and the whole file comes.
        let window = 256 << 10;
        let read = |offset: u64, length: u64| ReadRequest {
            path: path(&file),
            offset,
            length,
            ..Default::default()
        };
        let Streamed { chunks, end } = stream_all(&owner, "fs.read", &read(0, 0), window).await;
        end.unwrap();
        let mut got = Vec::new();
        for chunk in &chunks {
            assert!(chunk.len() <= window as usize, "a chunk of {}", chunk.len());
            let data = FsData::decode(chunk.clone()).unwrap();
            assert_eq!((data.total_size, data.file_size), (4 << 20, 4 << 20));
            got.extend_from_slice(&data.data);
        }
        assert!(got == content, "the file arrives whole");

        // A part of it.
        let Streamed { chunks, end } = stream_all(&owner, "fs.read", &read(10, 5), 0).await;
        end.unwrap();
        let part: Vec<u8> = chunks
            .into_iter()
            .flat_map(|chunk| FsData::decode(chunk).unwrap().data.to_vec())
            .collect();
        assert_eq!(part, content[10..15]);

        // A failed read ends with an error.
        let error = stream_all(
            &owner,
            "fs.read",
            &ReadRequest {
                path: path(&server.dir),
                ..Default::default()
            },
            0,
        )
        .await
        .end
        .unwrap_err();
        assert_eq!(error.code, codes::BAD_REQUEST);
        assert!(error.message.contains("is a directory"), "{error}");

        // A refused write: the chunk and every later one of it get the error,
        // and the file stays as it was.
        let refused = |last: bool| WriteRequest {
            transfer_id: 9,
            path: path(&file),
            total_size: 2,
            data: Bytes::from_static(b"x"),
            last,
            ..Default::default()
        };
        for last in [false, true] {
            let error = bus_services::call::<_, FsAck>(&owner, "fs.write", &refused(last))
                .await
                .unwrap_err();
            assert!(error.message.contains("already exists"), "{error}");
        }
        assert!(std::fs::read(&file).unwrap() == content);
    }

    #[tokio::test]
    async fn a_program_runs_on_the_bus_and_its_outputs_come_apart() {
        let server = start();
        let owner = server.owner().await;
        let done = run(&owner, &["/bin/sh", "-c", "echo out; echo err >&2; exit 3"]).await;
        assert!(done.exec_id > 0 && done.pid > 0);
        assert_eq!(done.stdout, b"out\n");
        assert_eq!(done.stderr, b"err\n");
        assert_eq!((done.exit.exec_id, done.exit.code), (done.exec_id, 3));

        // Stdin, acknowledged once the program took it.
        let mut stream = owner
            .stream(
                "exec.run",
                RunRequest {
                    argv: vec!["cat".into()],
                    stdin: true,
                    ..Default::default()
                }
                .encode_to_vec(),
                StreamOptions::default(),
            )
            .await
            .unwrap();
        let (exec_id, _) = started(&mut stream).await;
        let reply: InputReply = bus_services::call(
            &owner,
            "exec.input",
            &InputRequest {
                exec_id,
                data: Bytes::from_static(b"typed\n"),
                eof: true,
            },
        )
        .await
        .unwrap();
        assert_eq!((reply.exec_id, reply.taken), (exec_id, 6));
        let rest = ran_after(read_all(stream).await, Some((exec_id, 0)));
        assert_eq!(rest.stdout, b"typed\n");
        assert_eq!(rest.exit.code, 0);

        // A signal, and what killed it.
        let mut stream = owner
            .stream(
                "exec.run",
                RunRequest {
                    argv: vec!["sleep".into(), "100".into()],
                    ..Default::default()
                }
                .encode_to_vec(),
                StreamOptions::default(),
            )
            .await
            .unwrap();
        let (exec_id, _) = started(&mut stream).await;
        let _: SignalReply = bus_services::call(
            &owner,
            "exec.signal",
            &SignalRequest {
                exec_id,
                signal: 15,
            },
        )
        .await
        .unwrap();
        let end = read_all(stream).await.end.unwrap();
        let exit = tilr_bus::exec::ExecExit::decode(end).unwrap();
        assert_eq!((exit.code, exit.signal), (0, 15));

        // Only the caller of exec.run feeds or signals it; a program that did
        // not start is an error.
        let other = server.owner().await;
        let error = bus_services::call::<_, SignalReply>(
            &other,
            "exec.signal",
            &SignalRequest {
                exec_id: done.exec_id,
                signal: 15,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, codes::NOT_FOUND, "{error}");
        let error = stream_all(
            &owner,
            "exec.run",
            &RunRequest {
                argv: vec!["/no/such/program".into()],
                ..Default::default()
            },
            0,
        )
        .await
        .end
        .unwrap_err();
        assert_eq!(error.code, codes::SPAWN_FAILED, "{error}");
    }

    #[tokio::test]
    async fn a_cancelled_program_ends_with_its_group() {
        let server = start();
        let owner = server.owner().await;
        let fifo = server.dir.join("held");
        let closed = fifo_closed(&fifo);
        // A child of the program holds the FIFO open, and says so: it goes with
        // the group.
        let script = format!(
            "{{ echo held; exec sleep 100; }} 3>'{}' & wait",
            fifo.display()
        );
        let mut stream = owner
            .stream(
                "exec.run",
                RunRequest {
                    argv: vec!["/bin/sh".into(), "-c".into(), script],
                    ..Default::default()
                }
                .encode_to_vec(),
                StreamOptions::default(),
            )
            .await
            .unwrap();
        let (exec_id, _) = started(&mut stream).await;
        let chunk = within(stream.next()).await.unwrap().unwrap();
        let held = tilr_bus::exec::RunChunk::decode(chunk.bytes().unwrap()).unwrap();
        assert_eq!(
            held.chunk,
            Some(tilr_bus::exec::run_chunk::Chunk::Output(
                tilr_bus::exec::ExecOutput {
                    exec_id,
                    stream: 1,
                    data: Bytes::from_static(b"held\n"),
                }
            ))
        );
        stream.cancel();
        within(closed).await.expect("the FIFO's reader is gone");
    }

    #[tokio::test]
    async fn server_commands_are_taken_on_the_unix_socket_only() {
        let server = start();
        let command = CommandRequest {
            argv: vec!["--version".into()],
            ..Default::default()
        };
        let tcp = server
            .tcp(TcpAuth::AccessToken(server.token()))
            .await
            .unwrap();
        let error = stream_all(&tcp, "server.command", &command, 0)
            .await
            .end
            .unwrap_err();
        assert_eq!(error.code, codes::DENIED, "{error}");
        assert!(error.message.contains("Unix socket"), "{error}");

        let owner = server.owner().await;
        let version = ran(stream_all(&owner, "server.command", &command, 0).await);
        assert_eq!(version.exit.code, 0);
        assert!(
            String::from_utf8_lossy(&version.stdout).contains("tilrd"),
            "{:?}",
            String::from_utf8_lossy(&version.stdout)
        );
    }

    #[tokio::test]
    async fn a_copy_in_a_pane_is_an_event_and_its_content_a_stream() {
        let server = start();
        let owner = server.owner().await;
        let mut events = owner.subscribe("pane.copied").await.unwrap();
        // OSC 52 with "hello".
        let pane = create(
            &owner,
            80,
            24,
            sh("printf '\\033]52;c;aGVsbG8=\\007'; exec sleep 1000"),
        )
        .await;
        let event = next_event(&mut events).await;
        assert_eq!(event.scope, format!("pane:{}", pane.pane_id));
        assert_eq!(event.from, "tilrd");
        let copied: Copied = event.decode().unwrap();
        assert_eq!(copied.pane_id, pane.pane_id);
        assert_eq!(copied.total_size, 5);
        assert!(copied.mime.starts_with("text/plain"), "{}", copied.mime);

        let get = |transfer_id| GetRequest { transfer_id };
        let Streamed { chunks, end } =
            stream_all(&owner, "clipboard.get", &get(copied.transfer_id), 0).await;
        end.unwrap();
        let first = ClipboardData::decode(chunks[0].clone()).unwrap();
        assert_eq!(
            (first.pane_id.as_str(), first.total_size),
            (pane.pane_id.as_str(), 5)
        );
        let content: Vec<u8> = chunks
            .into_iter()
            .flat_map(|chunk| ClipboardData::decode(chunk).unwrap().data.to_vec())
            .collect();
        assert_eq!(content, b"hello");

        // A newer copy replaces it.
        create(
            &owner,
            80,
            24,
            sh("printf '\\033]52;c;d29ybGQ=\\007'; exec sleep 1000"),
        )
        .await;
        let newer: Copied = next_event(&mut events).await.decode().unwrap();
        assert!(newer.transfer_id > copied.transfer_id);
        let error = stream_all(&owner, "clipboard.get", &get(copied.transfer_id), 0)
            .await
            .end
            .unwrap_err();
        assert_eq!(error.code, codes::NOT_FOUND, "{error}");
    }

    #[tokio::test]
    async fn the_shared_clipboard_takes_an_upload_in_chunks() {
        let server = start();
        let owner = server.owner().await;
        let put = |transfer_id, mime: &str, total_size, data: &'static [u8], last| PutRequest {
            transfer_id,
            mime: mime.into(),
            total_size,
            data: Bytes::from_static(data),
            last,
        };
        let reply: PutReply = bus_services::call(
            &owner,
            "clipboard.put",
            &put(1, "text/plain", 10, b"hello", false),
        )
        .await
        .unwrap();
        assert_eq!(reply.transfer_id, 1);
        let _: PutReply =
            bus_services::call(&owner, "clipboard.put", &put(1, "", 0, b"world", true))
                .await
                .unwrap();
        // Readable by the panes' xclip once the last chunk is answered.
        let pane = create(
            &owner,
            80,
            24,
            sh("xclip -selection clipboard -o; echo; exec sleep 1000"),
        )
        .await;
        wait_text(&owner, &pane.pane_id, |lines| {
            lines.iter().any(|line| line == "helloworld")
        })
        .await;

        // Over the limit: refused, and the rest of that transfer too.
        let info: InfoReply = bus_services::call(&owner, "server.info", &InfoRequest {})
            .await
            .unwrap();
        for (mime, last) in [("text/plain", false), ("", true)] {
            let error = bus_services::call::<_, PutReply>(
                &owner,
                "clipboard.put",
                &put(2, mime, info.clipboard_max_bytes + 1, b"x", last),
            )
            .await
            .unwrap_err();
            assert!(error.message.contains("too large"), "{error}");
        }
    }
}
