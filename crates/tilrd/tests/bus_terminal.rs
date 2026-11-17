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
    Screen, Watching, call, create, create_shell, eventually, list, next_reply, raw,
    raw_with_client_id, read, send_call, send_call_bytes, shell, wait_text,
};
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
use tilr_bus::{Bus, BusError, ConnectOptions, codes};

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

async fn file_is(path: &std::path::Path, expected: &str) {
    eventually(
        &format!("{} holds {expected:?}", path.display()),
        || async { std::fs::read_to_string(path).is_ok_and(|text| text == expected) },
    )
    .await;
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
    drop(watching);
    eventually("the stream's caller stopped viewing", || async {
        let panes = list(&owner).await;
        panes[0].viewers == 1
            && !panes[0]
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
    laggard
        .send(Msg::Call(Call {
            id: 1,
            method: "pane.output".into(),
            payload: OutputRequest {
                pane_id: info.pane_id.clone(),
                ..Default::default()
            }
            .encode_to_vec()
            .into(),
            window: 64 << 10,
            ..Call::default()
        }))
        .await;
    let mut screen = Screen::default();
    let chunk = async |laggard: &mut common::bus::Raw, screen: &mut Screen, wait: Duration| {
        let msg = laggard.recv_within(wait).await?;
        match msg {
            Msg::Chunk(chunk) => {
                assert_eq!(chunk.id, 1);
                let bytes = chunk.payload.len() as u32;
                let payload = tilr_bus::Chunk {
                    payload: chunk.payload,
                    content_encoding: chunk.content_encoding,
                }
                .bytes()
                .unwrap();
                let frame = screen.apply_chunk(&payload);
                laggard.send(Msg::Credit(Credit { id: 1, bytes })).await;
                Some(frame.is_some_and(|frame| frame.full))
            }
            other => panic!("expected a chunk of pane.output, got {other:?}"),
        }
    };
    assert_eq!(chunk(&mut laggard, &mut screen, TIMEOUT).await, Some(true));

    // The laggard reads nothing while the pane prints for 3 s.
    raw(
        &owner,
        &info.pane_id,
        "end=$(($(date +%s)+3)); while [ $(date +%s) -lt $end ]; do seq 1 20000; done; echo fin''ished\n",
    )
    .await
    .unwrap();
    wait_text(&owner, &info.pane_id, |lines| {
        let last = lines.iter().rposition(|line| !line.is_empty());
        last.is_some_and(|last| last > 0 && lines[last] == "$" && lines[last - 1] == "finished")
    })
    .await;

    // It reads again: a full frame of the screen as it is now, and it was not
    // dropped for lagging.
    while chunk(&mut laggard, &mut screen, Duration::from_millis(500))
        .await
        .is_some()
    {}
    let now = read(
        &owner,
        &ReadRequest {
            pane_id: info.pane_id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(screen.rows, now.lines);
    assert!(
        screen.full_frames >= 2,
        "the laggard caught up with a full frame ({} frames, {} full)",
        screen.frames,
        screen.full_frames
    );
    assert!(laggard.close.is_none());
    assert_eq!(list(&owner).await[0].viewers, 1);
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
    let out = server.dir.join("order.txt");
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", out.display())),
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
    file_is(&out, &expected).await;

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
    let out = server.dir.join("numbered.txt");
    let pane = create(
        &anonymous,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", out.display())),
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
    file_is(&out, "1\n2\n").await;
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
    file_is(&out, "1\n2\n4\n").await;
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
            break;
        }
    }
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

    // Nothing changes: the same seq once the wait is over.
    let quiet = wait_text(&owner, &info.pane_id, |lines| {
        lines.get(1).is_some_and(|line| line == "typed")
            && lines.get(2).is_some_and(|line| line == "$")
    })
    .await;
    let started = std::time::Instant::now();
    let unchanged = read_now(ReadRequest {
        pane_id: info.pane_id.clone(),
        since_seq: Some(quiet.seq),
        wait_ms: Some(300),
        ..Default::default()
    })
    .await;
    let waited = started.elapsed();
    assert_eq!(unchanged.seq, quiet.seq);
    assert_eq!(unchanged.lines, quiet.lines);
    assert!(
        waited >= Duration::from_millis(280) && waited < Duration::from_secs(4),
        "{waited:?}"
    );

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
    let typed = server.dir.join("typed.txt");
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", typed.display())),
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
    file_is(&typed, "kpasted\n").await;

    // A mouse press, once the program asked for SGR mouse reports.
    let clicked = server.dir.join("clicked.txt");
    let mouse = create(
        &owner,
        80,
        24,
        sh(&format!(
            "stty -echo raw; printf '\\033[?1000h\\033[?1006hready'; head -c 9 > '{}'",
            clicked.display()
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
    file_is(&clicked, "\x1b[<0;4;3M").await;

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
    let (payload, tail) = (server.dir.join("payload"), server.dir.join("tail"));
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
            tail.display()
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
    file_is(&tail, "END\n").await;
    assert_eq!(std::fs::read(&payload).unwrap(), vec![b'a'; 1 << 20]);
}

#[tokio::test]
async fn internal_fields_cannot_be_forged() {
    let server = start();
    let owner = server.owner().await;
    let out = server.dir.join("forged.txt");
    let pane = create(
        &owner,
        80,
        24,
        sh(&format!("stty -echo; cat > '{}'", out.display())),
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
    file_is(&out, "v\nx\n").await;
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

    // No answer from the pane process: unavailable, not a call that hangs.
    let request = ReadRequest {
        pane_id: info.pane_id.clone(),
        ..Default::default()
    };
    let error = read(&owner, &request).await.unwrap_err();
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
    // It prints for 2 s once told to, then its last words, and exits.
    let info = create(
        &owner,
        100,
        30,
        sh("read go; end=$(($(date +%s)+2)); while [ $(date +%s) -lt $end ]; do seq 1 20000; done; echo last''words; exit 4"),
    )
    .await;
    let mut laggard = server.raw_greeted("laggard", Vec::new()).await;
    send_call(
        &mut laggard,
        1,
        "pane.output",
        &OutputRequest {
            pane_id: info.pane_id.clone(),
            ..Default::default()
        },
    )
    .await;
    let mut screen = Screen::default();
    let mut chunk = async |laggard: &mut common::bus::Raw| match laggard.recv().await {
        Some(Msg::Chunk(chunk)) => {
            let payload = tilr_bus::Chunk {
                payload: chunk.payload,
                content_encoding: chunk.content_encoding,
            }
            .bytes()
            .unwrap();
            screen.apply_chunk(&payload);
            None
        }
        Some(Msg::End(end)) => Some(PaneExited::decode(end.payload).unwrap()),
        other => panic!("expected pane.output, got {other:?}"),
    };
    assert!(chunk(&mut laggard).await.is_none());

    // The laggard reads nothing until the pane has exited.
    raw(&owner, &info.pane_id, "\n").await.unwrap();
    tokio::time::timeout(TIMEOUT, events.next())
        .await
        .expect("the pane exits")
        .expect("the subscription ended");
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

#[cfg(feature = "failpoints")]
mod failpoints {
    use super::*;
    use common::PauseController;

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
        let out = server.dir.join("lost.txt");
        let pane = create(
            &owner,
            80,
            24,
            sh(&format!("stty -echo; cat > '{}'", out.display())),
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
        file_is(&out, "1\n2\n").await;
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
