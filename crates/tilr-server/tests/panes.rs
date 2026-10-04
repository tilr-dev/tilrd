//! Use cases: connecting, and running a shell in a pane.

mod common;

use common::*;

#[tokio::test]
async fn hello_announces_the_clipboard_and_its_limit() {
    let server = Server::start(&["--clipboard-max", "3MB"]);
    let mut client = Client::connect(&server).await;
    let ack = client.hello().await;
    assert_eq!(ack.protocol_version, PROTOCOL_VERSION);
    assert_eq!(ack.features, ["clipboard", "ping"]);
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
async fn a_browser_page_connects_only_from_an_allowed_origin() {
    let server = Server::start(&["--allow-origin", "http://app.test"]);
    // A page from anywhere else must not get a shell on this machine.
    let refused = Client::connect_from(&server, "http://evil.test").await;
    assert!(refused.is_err_and(|err| err.contains("403")));

    let mut page = Client::connect_from(&server, "http://app.test")
        .await
        .unwrap();
    assert_eq!(page.hello().await.protocol_version, PROTOCOL_VERSION);
    // Programs that are not browsers send no origin and are let in.
    let mut program = Client::connect(&server).await;
    assert_eq!(program.hello().await.protocol_version, PROTOCOL_VERSION);
}

#[tokio::test]
async fn a_malformed_packet_gets_an_error_and_the_connection_stays() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    // Flag byte 9 does not exist.
    client
        .ws
        .send(Message::Binary(Bytes::from_static(&[9, 1, 2])))
        .await
        .unwrap();
    assert!(client.error().await.contains("bad packet"));
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn asking_for_a_pane_that_does_not_exist_gets_an_error() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    client.subscribe(999).await;
    assert_eq!(client.error().await, "no such pane: 999");
    client.type_line(999, "echo").await;
    assert_eq!(client.error().await, "no such pane: 999");
}

#[tokio::test]
async fn two_clients_watch_and_type_into_the_same_pane() {
    let server = Server::start(&[]);
    let mut first = Client::connect(&server).await;
    let pane_id = first.open_pane().await;
    let mut second = Client::connect(&server).await;
    second.subscribe(pane_id).await;

    first
        .type_line(pane_id, "echo \"from the fi\"\"rst\"")
        .await;
    first.wait_row("from the first").await;
    second.wait_row("from the first").await;

    second
        .type_line(pane_id, "echo \"from the sec\"\"ond\"")
        .await;
    first.wait_row("from the second").await;
    second.wait_row("from the second").await;

    assert_eq!(first.list_panes().await[0].viewers, 2);
}

#[tokio::test]
async fn a_client_that_leaves_does_not_take_the_pane_with_it() {
    let server = Server::start(&[]);
    let mut stays = Client::connect(&server).await;
    let pane_id = stays.open_pane().await;
    let mut leaves = Client::connect(&server).await;
    leaves.subscribe(pane_id).await;
    leaves
        .wait_for(|msg| matches!(msg, server_message::Msg::Frame(_)).then_some(()))
        .await;
    assert_eq!(stays.list_panes().await[0].viewers, 2);

    drop(leaves);
    // The server tells the remaining client on its own.
    loop {
        let panes = stays.pushed_pane_list().await;
        if panes[0].viewers == 1 {
            break;
        }
    }
    assert_eq!(stays.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_client_that_stops_watching_gets_no_more_frames() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);

    client.send(Msg::Unsubscribe(Unsubscribe { pane_id })).await;
    assert_eq!(client.list_panes().await[0].viewers, 0);
    client.frame = None;
    client.type_line(pane_id, "echo more output").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client.list_panes().await;
    assert!(client.frame.is_none());
}

#[tokio::test]
async fn resizing_a_pane_resizes_the_terminal_the_shell_sees() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    client
        .send(Msg::Resize(Resize {
            pane_id,
            cols: 50,
            rows: 10,
        }))
        .await;
    // A resize redraws the whole screen at the new size.
    let frame = client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) if frame.cols == 50 => Some(frame.clone()),
            _ => None,
        })
        .await;
    assert!(frame.full);
    assert_eq!(frame.rows, 10);

    let out = server.dir.join("size");
    assert_eq!(
        client
            .run(pane_id, &format!("stty size > '{}'", out.display()))
            .await,
        0
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "10 50\n");
    let panes = client.list_panes().await;
    assert_eq!((panes[0].cols, panes[0].rows), (50, 10));
}

#[tokio::test]
async fn a_client_built_for_another_protocol_version_is_turned_away() {
    let server = Server::start(&[]);
    // Newer than the server, older than it, and one that sent no version.
    for version in [PROTOCOL_VERSION + 1, PROTOCOL_VERSION - 1, 0] {
        if version == PROTOCOL_VERSION {
            continue;
        }
        let mut client = Client::connect_silent(&server).await;
        client.hello_as(version).await;
        // The first thing it hears is why, with both versions; then nothing.
        let Some(server_message::Msg::Error(error)) = client.next().await else {
            panic!("version {version} was not refused");
        };
        assert_eq!(
            error.message,
            format!(
                "unsupported protocol version {version}: this server speaks {PROTOCOL_VERSION}"
            )
        );
        assert!(client.next().await.is_none());
    }
    // The server carries on for those who speak its version.
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn a_client_that_skips_hello_is_turned_away() {
    let server = Server::start(&[]);
    let mut asks_first = Client::connect(&server).await;
    let pane_id = asks_first.open_pane().await;

    let mut client = Client::connect_silent(&server).await;
    client.type_line(pane_id, "echo \"got thr\"\"ough\"").await;
    let Some(server_message::Msg::Error(error)) = client.next().await else {
        panic!("a client that did not say hello was not refused");
    };
    assert_eq!(error.message, "expected Hello first");
    assert!(client.next().await.is_none());

    // What it sent did not reach the shell.
    assert_eq!(asks_first.run(pane_id, "true").await, 0);
    assert!(!asks_first.screen.has_row("got through"));
}
