//! Acknowledged input survives disconnects and exec without duplicate PTY bytes.
#![cfg(feature = "failpoints")]
mod common;
use common::*;
use tilr_proto::input::Outbox;

fn server() -> Server {
    let dir = Server::scratch_dir();
    Server::start_failing(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), &[], "")
}

async fn identify(server: &Server) -> (Client, HelloAck) {
    let mut client = Client::connect_silent(server).await;
    client.client_id = "input-session".into();
    let hello = client.hello().await;
    (client, hello)
}

async fn delivery_responses(client: &mut Client) -> (Vec<String>, Vec<tilr_proto::InputAck>) {
    client.ping(315).await;
    let mut errors = Vec::new();
    let mut acknowledgements = Vec::new();
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Error(error) => {
                errors.push(error.message.clone());
                None
            }
            server_message::Msg::InputAck(ack) => {
                acknowledgements.push(ack.clone());
                None
            }
            // Supervisor replies before processing this ordered barrier.
            server_message::Msg::Pong(pong) if pong.nonce == 315 => Some(()),
            _ => None,
        })
        .await;
    (errors, acknowledgements)
}

// @spec protocol/missing-pane-input-receives-a-terminal-acknowledgement
#[tokio::test]
async fn missing_pane_input_is_rejected_and_reclaimed_across_reconnect() {
    let server = server();
    let (mut client, _) = identify(&server).await;
    let live = client.open_pane().await;
    let mut queue = Outbox::default();
    // Retain an unsent live-pane entry to prove rejection is scoped by pane.
    queued(&mut queue, live, b"live");
    let missing = uuid::Uuid::new_v4().to_string();
    let mut expected = Vec::new();
    for named in [missing.as_str(), "not-a-pane"] {
        for msg in [
            Msg::Key(KeyInput {
                pane_id: named.into(),
                ..Default::default()
            }),
            Msg::Mouse(tilr_proto::MouseInput {
                pane_id: named.into(),
                ..Default::default()
            }),
            Msg::Paste(tilr_proto::Paste {
                pane_id: named.into(),
                ..Default::default()
            }),
            Msg::Raw(RawInput {
                pane_id: named.into(),
                ..Default::default()
            }),
            Msg::Scroll(Scroll {
                pane_id: named.into(),
                ..Default::default()
            }),
            Msg::Resize(Resize {
                pane_id: named.into(),
                cols: 80,
                rows: 24,
                ..Default::default()
            }),
        ] {
            let message = queue.push(msg.into()).unwrap();
            expected.push(tilr_proto::InputAck {
                pane_id: named.into(),
                seq: message.input_seq,
            });
            send(&mut client, &message).await;
        }
    }
    let (errors, acks) = delivery_responses(&mut client).await;
    assert_eq!(
        errors,
        expected
            .iter()
            .map(|a| format!("no such pane: {}", a.pane_id))
            .collect::<Vec<_>>()
    );
    // Intentionally lose the first acknowledgements: all rejected entries
    // remain retained and must get the same terminal response on replay.
    assert_eq!(acks, expected);
    drop(client);
    let (mut client, hello) = resume(&server, &mut queue).await;
    assert!(
        hello
            .applied_input
            .iter()
            .all(|a| a.pane_id == live.to_string())
    );
    let (errors, acks) = delivery_responses(&mut client).await;
    assert_eq!(errors.len(), expected.len());
    let rejected: Vec<_> = acks
        .iter()
        .filter(|a| a.pane_id != live.to_string())
        .cloned()
        .collect();
    assert_eq!(rejected, expected);
    // The live pane may answer asynchronously; only consume the rejections.
    for ack in &rejected {
        queue.acknowledge(ack);
    }
    assert_eq!(queue.pending().count(), 1);
    assert_eq!(
        tilr_proto::input::pane(queue.pending().next().unwrap().msg.as_ref().unwrap()),
        Some(live.to_string().as_str())
    );
    let live_ack = match acks.iter().find(|a| a.pane_id == live.to_string()) {
        Some(ack) => ack.clone(),
        None => ack(&mut client, live, 1).await,
    };
    queue.acknowledge(&live_ack);
    assert_eq!(queue.pending().count(), 0);
    // Reclaimed capacity admits new input and keeps the per-pane counter.
    let next = queue
        .push(
            Msg::Raw(RawInput {
                pane_id: missing.clone(),
                ..Default::default()
            })
            .into(),
        )
        .unwrap();
    assert_eq!(next.input_seq, 7);
    send(&mut client, &next).await;
    let (_, acks) = delivery_responses(&mut client).await;
    for ack in &acks {
        queue.acknowledge(ack);
    }
    assert_eq!(queue.pending().count(), 0);
    drop(client);
    let (mut client, hello) = resume(&server, &mut queue).await;
    assert!(
        hello
            .applied_input
            .iter()
            .all(|a| a.pane_id == live.to_string())
    );
    let (errors, acks) = delivery_responses(&mut client).await;
    assert!(errors.is_empty());
    assert!(acks.is_empty());

    // Neither unnumbered input nor non-input controls acquire delivery acks.
    client
        .send(Msg::Raw(RawInput {
            pane_id: missing.clone(),
            ..Default::default()
        }))
        .await;
    client
        .send(Msg::Subscribe(Subscribe {
            pane_id: missing.clone(),
        }))
        .await;
    let (errors, acks) = delivery_responses(&mut client).await;
    assert_eq!(errors, vec![format!("no such pane: {missing}"); 2]);
    assert!(acks.is_empty());
}

#[tokio::test]
async fn a_pending_hello_is_superseded_before_the_pane_answers() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let mut owner = Client::connect(&server).await;
    owner.open_pane().await;
    pause(&server, "input-before-probe");
    let mut old = Client::connect_silent(&server).await;
    old.client_id = "input-session".into();
    old.hello_as(PROTOCOL_VERSION).await;
    let hit = pauses.wait("input-before-probe").await;
    let mut new = Client::connect_silent(&server).await;
    new.client_id = old.client_id.clone();
    new.hello_as(PROTOCOL_VERSION).await;
    // Superseding must finish while the first handshake is still blocked.
    let closed = tokio::time::timeout(TIMEOUT, async {
        while let Some(Ok(message)) = old.ws.next().await {
            if let Message::Close(Some(frame)) = message {
                return u16::from(frame.code);
            }
        }
        panic!("missing superseded close frame");
    })
    .await
    .expect("pending handshake was left open");
    assert_eq!(closed, tilr_proto::close::SUPERSEDED);
    clear(&server);
    hit.release().await;
    new.wait_for(|msg| match msg {
        server_message::Msg::HelloAck(ack) => Some(ack.clone()),
        _ => None,
    })
    .await;
    assert_eq!(new.list_panes().await.len(), 1);
}

// @spec protocol/hello-input-probes-are-bounded
#[tokio::test]
async fn a_stuck_pane_does_not_block_other_panes_hello() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let (mut client, _) = identify(&server).await;
    let healthy = client.open_pane().await;
    let file = collector(&server, &mut client, healthy).await;
    let stuck = client.open_pane().await;
    let mut queue = Outbox::default();
    let first = queued(&mut queue, healthy, b"first\n");
    send(&mut client, &first).await;
    ack(&mut client, healthy, 1).await;
    drop(client); // Keep the applied input in the outbox as if its ack was lost.

    let point = format!("input-before-probe:{stuck}");
    pause(&server, &point);
    let mut returning = Client::connect_silent(&server).await;
    returning.client_id = "input-session".into();
    returning.hello_as(PROTOCOL_VERSION).await;
    let held = pauses.wait(&point).await;
    let mut other = Client::connect_silent(&server).await;
    other.client_id = "another-input-session".into();
    other.hello_as(PROTOCOL_VERSION).await;
    let hello = returning
        .wait_for(|msg| match msg {
            server_message::Msg::HelloAck(hello) => Some(hello.clone()),
            _ => None,
        })
        .await;
    let other_hello = other
        .wait_for(|msg| match msg {
            server_message::Msg::HelloAck(hello) => Some(hello.clone()),
            _ => None,
        })
        .await;
    assert_eq!(hello.applied_input.len(), 1);
    assert_eq!(hello.applied_input[0].pane_id, healthy.to_string());
    assert_eq!(hello.applied_input[0].seq, 1);
    assert_eq!(other_hello.applied_input.len(), 1);
    assert_eq!(other_hello.applied_input[0].pane_id, healthy.to_string());
    assert_eq!(other_hello.applied_input[0].seq, 0);
    let panes = returning.list_panes().await;
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].pane_id, healthy.to_string());
    clear(&server);
    drop(held); // Its pane has exited; the controller must not release it.

    returning.subscribe(healthy).await;
    queue.acknowledge(&hello.applied_input[0]);
    assert_eq!(queue.pending().count(), 0);
    send(&mut returning, &first).await; // A stale replay must still be harmless.
    ack(&mut returning, healthy, 1).await;
    let second = queued(&mut queue, healthy, b"second\n");
    send(&mut returning, &second).await;
    ack(&mut returning, healthy, 2).await;
    finish(&mut returning, healthy, &file).await;
}

fn queued(queue: &mut Outbox, pane: PaneId, data: &[u8]) -> ClientMessage {
    queue
        .push(
            Msg::Raw(RawInput {
                pane_id: pane.to_string(),
                data: Bytes::copy_from_slice(data),
            })
            .into(),
        )
        .unwrap()
}
async fn send(client: &mut Client, message: &ClientMessage) {
    client
        .ws
        .send(Message::Binary(tilr_proto::encode(message)))
        .await
        .unwrap();
}
async fn ack(client: &mut Client, pane: PaneId, seq: u64) -> tilr_proto::InputAck {
    client
        .wait_for(|msg| match msg {
            server_message::Msg::InputAck(ack)
                if ack.pane_id == pane.to_string() && ack.seq == seq =>
            {
                Some(ack.clone())
            }
            _ => None,
        })
        .await
}
async fn resume(server: &Server, queue: &mut Outbox) -> (Client, HelloAck) {
    let (mut client, hello) = identify(server).await;
    for applied in &hello.applied_input {
        queue.acknowledge(applied);
    }
    for message in queue.pending() {
        send(&mut client, message).await;
    }
    (client, hello)
}
fn pause(server: &Server, name: &str) {
    std::fs::write(server.dir.join("failpoints"), format!("{name}=pause")).unwrap();
}
fn clear(server: &Server) {
    std::fs::write(server.dir.join("failpoints"), "").unwrap();
}
async fn collector(server: &Server, client: &mut Client, pane: PaneId) -> PathBuf {
    let file = server.dir.join("typed");
    client.type_line(pane, &format!("stty -echo; printf 'collector-%s\\n' ready; cat > '{}'; printf 'collector-%s\\n' finished", file.display())).await;
    client.wait_screen("collector-ready").await;
    file
}
async fn finish(client: &mut Client, pane: PaneId, file: &Path) {
    client
        .send(Msg::Raw(RawInput {
            pane_id: pane.to_string(),
            data: Bytes::from_static(b"\x04"),
        }))
        .await;
    client.wait_screen("collector-finished").await;
    assert_eq!(std::fs::read(file).unwrap(), b"first\nsecond\n");
}

// @spec protocol/input-is-acknowledged-after-application
// @spec reload/client-connections-end-and-clients-reconnect
#[tokio::test]
async fn unread_input_is_replayed_after_reload_in_the_middle_of_a_batch() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let (mut client, _) = identify(&server).await;
    let pane = client.open_pane().await;
    let file = collector(&server, &mut client, pane).await;
    let mut queue = Outbox::default();
    let first = queued(&mut queue, pane, b"first\n");
    send(&mut client, &first).await;
    queue.acknowledge(&ack(&mut client, pane, 1).await);
    pause(&server, "reload-after-drain");
    server.signal("-USR1");
    let hit = pauses.wait("reload-after-drain").await;
    // The supervisor has made its last read. This successful socket write
    // cannot be included in the state handed to the successor.
    let second = queued(&mut queue, pane, b"second\n");
    send(&mut client, &second).await;
    clear(&server);
    hit.release().await;
    let mut client = reconnect(&server, client).await;
    drop(client);
    let (fresh, hello) = resume(&server, &mut queue).await;
    client = fresh;
    assert_eq!(
        hello
            .applied_input
            .iter()
            .find(|a| a.pane_id == pane.to_string())
            .unwrap()
            .seq,
        1
    );
    client.subscribe(pane).await;
    queue.acknowledge(&ack(&mut client, pane, 2).await);
    assert_eq!(queue.pending().count(), 0);
    finish(&mut client, pane, &file).await;
}

// @spec protocol/input-is-acknowledged-after-application
#[tokio::test]
async fn applied_input_with_a_lost_ack_is_not_applied_twice() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let (mut client, _) = identify(&server).await;
    let pane = client.open_pane().await;
    let file = collector(&server, &mut client, pane).await;
    let mut queue = Outbox::default();
    pause(&server, "input-before-ack");
    let first = queued(&mut queue, pane, b"first\n");
    send(&mut client, &first).await;
    let hit = pauses.wait("input-before-ack").await;
    // The PTY already owns these bytes. Close before any acknowledgement.
    drop(client);
    clear(&server);
    hit.release().await;
    let (mut client, hello) = resume(&server, &mut queue).await;
    assert_eq!(
        hello
            .applied_input
            .iter()
            .find(|a| a.pane_id == pane.to_string())
            .unwrap()
            .seq,
        1
    );
    assert_eq!(queue.pending().count(), 0);
    // Even a stale replay must be harmless.
    send(&mut client, &first).await;
    ack(&mut client, pane, 1).await;
    let second = queued(&mut queue, pane, b"second\n");
    send(&mut client, &second).await;
    ack(&mut client, pane, 2).await;
    client.subscribe(pane).await;
    finish(&mut client, pane, &file).await;
}

// @spec protocol/input-is-acknowledged-after-application
#[tokio::test]
async fn disconnect_before_application_replays_one_copy() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let (mut client, _) = identify(&server).await;
    let pane = client.open_pane().await;
    let file = collector(&server, &mut client, pane).await;
    let mut queue = Outbox::default();
    pause(&server, "input-before-apply");
    let first = queued(&mut queue, pane, b"first\n");
    send(&mut client, &first).await;
    let hit = pauses.wait("input-before-apply").await;
    drop(client);
    clear(&server);
    hit.release().await;
    let (mut client, _) = resume(&server, &mut queue).await;
    let second = queued(&mut queue, pane, b"second\n");
    send(&mut client, &second).await;
    ack(&mut client, pane, 2).await;
    client.subscribe(pane).await;
    finish(&mut client, pane, &file).await;
}

// @spec protocol/input-is-acknowledged-after-application
// @spec reload/pane-input-and-output-are-not-lost-across-a-reload
#[tokio::test]
async fn partially_written_input_keeps_its_boundary_through_exec() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let (fifo, mut release) = activity_fifo(&server);
    let (mut client, _) = identify(&server).await;
    let pane = client.open_pane().await;
    let file = server.dir.join("large-input");
    let tail = server.dir.join("after-input");
    client.type_line(pane, &format!(
        "stty -echo -icanon min 1 time 0; printf 'reader-%s\\n' ready; read -r _ < '{}'; head -c 1048576 > '{}'; head -c 4 > '{}'; printf 'reader-%s\\n' finished",
        fifo.display(), file.display(), tail.display()
    )).await;
    client.wait_screen("reader-ready").await;
    let mut queue = Outbox::default();
    pause(&server, "input-before-apply");
    let message = queued(&mut queue, pane, &vec![b'a'; 1048576]);
    send(&mut client, &message).await;
    let hit = pauses.wait("input-before-apply").await;
    clear(&server);
    hit.release().await;
    drop(client);
    // A probe is ordered after acceptance; the blocked reader prevents the
    // whole frame fitting in the PTY. No applied ACK is allowed yet.
    let (mut client, hello) = resume(&server, &mut queue).await;
    assert_eq!(
        hello
            .applied_input
            .iter()
            .find(|a| a.pane_id == pane.to_string())
            .unwrap()
            .seq,
        0
    );
    pause(&server, "reload-after-drain");
    server.signal("-USR1");
    let hit = pauses.wait("reload-after-drain").await;
    clear(&server);
    hit.release().await;
    client = reconnect(&server, client).await;
    drop(client);
    let (mut client, hello) = resume(&server, &mut queue).await;
    assert_eq!(
        hello
            .applied_input
            .iter()
            .find(|a| a.pane_id == pane.to_string())
            .unwrap()
            .seq,
        0
    );
    client.subscribe(pane).await;
    release_activity_fifo(&mut release);
    ack(&mut client, pane, 1).await;
    let end = queued(&mut queue, pane, b"END\n");
    send(&mut client, &end).await;
    ack(&mut client, pane, 2).await;
    client.wait_screen("reader-finished").await;
    assert_eq!(std::fs::read(file).unwrap(), vec![b'a'; 1048576]);
    assert_eq!(std::fs::read(tail).unwrap(), b"END\n");
}

#[tokio::test]
async fn an_old_input_ack_cannot_leave_a_later_unnumbered_resize_stale() {
    let server = server();
    let pauses = PauseController::bind(&server.dir);
    let (mut client, _) = identify(&server).await;
    let pane = client.open_pane().await;
    let mut queue = Outbox::default();
    pause(&server, "input-before-ack");
    send(&mut client, &queued(&mut queue, pane, b"")).await;
    let hit = pauses.wait("input-before-ack").await;
    client
        .send(Msg::Resize(Resize {
            pane_id: pane.to_string(),
            cols: 120,
            rows: 40,
            ..Default::default()
        }))
        .await;
    let info = client.list_panes().await;
    assert_eq!((info[0].cols, info[0].rows), (120, 40));
    clear(&server);
    hit.release().await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) if frame.cols == 120 && frame.rows == 40 => Some(()),
            _ => None,
        })
        .await;
    let info = client.list_panes().await;
    assert_eq!((info[0].cols, info[0].rows), (120, 40));
}

// @spec protocol/input-is-acknowledged-after-application
#[tokio::test]
async fn numbered_input_requires_a_stable_client_id() {
    let server = server();
    let mut client = Client::connect(&server).await;
    assert!(client.client_id.is_empty());
    let pane = client.open_pane().await;
    let mut queue = Outbox::default();
    send(
        &mut client,
        &queued(&mut queue, pane, b"must not be applied\n"),
    )
    .await;
    assert_eq!(
        client.error().await,
        "numbered input requires a stable client_id"
    );
}

// @spec protocol/hello-comes-first
#[tokio::test]
async fn numbered_input_before_hello_is_refused_and_closed() {
    let server = server();
    let mut client = Client::connect_silent(&server).await;
    let mut queue = Outbox::default();
    send(
        &mut client,
        &queued(&mut queue, uuid::Uuid::new_v4(), b"ignored"),
    )
    .await;
    assert_eq!(client.error().await, "expected Hello first");
    let closed = tokio::time::timeout(TIMEOUT, client.ws.next())
        .await
        .unwrap();
    assert!(
        matches!(closed, Some(Ok(Message::Close(_))) | None),
        "{closed:?}"
    );
}
