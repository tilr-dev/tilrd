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
    assert!(server.logged("client went silent"));
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
    assert!(client.hello().await.features.contains(&"ping".to_owned()));
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
