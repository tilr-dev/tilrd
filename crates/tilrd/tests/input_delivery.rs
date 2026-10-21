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

// @spec panes/applied-resize-dimensions-precede-delayed-input-acknowledgement
#[tokio::test]
async fn a_numbered_resize_updates_the_pane_list_while_pty_input_waits() {
    let server = server();
    let (fifo, mut release) = activity_fifo(&server);
    let (mut client, hello) = identify(&server).await;
    let pane = client.open_pane().await;
    client.type_line(pane, &format!(
        "stty -echo -icanon min 1 time 0; printf 'reader-%s\\n' ready; read -r _ < '{}'; head -c 1048576 > /dev/null; printf 'reader-%s\\n' finished",
        fifo.display()
    )).await;
    client.wait_screen("reader-ready").await;
    let mut queue = Outbox::default();
    send(&mut client, &queued(&mut queue, pane, &vec![b'a'; 1048576])).await;
    let mut resize: ClientMessage = Msg::Resize(Resize {
        pane_id: pane.to_string(),
        cols: 120,
        rows: 40,
        ..Default::default()
    })
    .into();
    resize.seq = hello.seq_base + 1;
    let resize = queue.push(resize).unwrap();
    send(&mut client, &resize).await;
    // The frame proves Resize was applied. Size notifications are flushed
    // before frames on the same pane-to-supervisor stream.
    let mut pushed_size = None;
    client
        .wait_for(|msg| {
            match msg {
                server_message::Msg::InputAck(ack) => assert_eq!(ack.seq, 0),
                server_message::Msg::PaneList(list) => {
                    let info = list
                        .panes
                        .iter()
                        .find(|info| info.pane_id == pane.to_string())
                        .unwrap();
                    pushed_size = Some((info.cols, info.rows));
                }
                server_message::Msg::Frame(frame) if frame.last_input_seq == hello.seq_base + 1 => {
                    assert_eq!((frame.cols, frame.rows), (120, 40));
                    return Some(());
                }
                _ => {}
            }
            None
        })
        .await;
    assert_eq!(pushed_size, Some((120, 40)));
    let info = client.list_panes().await;
    assert_eq!((info[0].cols, info[0].rows), (120, 40));
    drop(client);
    let (mut client, hello) = identify(&server).await;
    assert_eq!(
        hello
            .applied_input
            .iter()
            .find(|ack| ack.pane_id == pane.to_string())
            .unwrap()
            .seq,
        0
    );
    release_activity_fifo(&mut release);
    ack(&mut client, pane, 2).await;
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
