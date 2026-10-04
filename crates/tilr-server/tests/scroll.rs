//! Use cases: looking back at what scrolled off the screen.
//!
//! Every test prints `line-1` to `line-200` into a pane 30 rows high, so the
//! screen ends on `line-200` and the rest is history.

mod common;

use common::*;

async fn pane_with_history(server: &Server) -> (Client, PaneId) {
    let mut client = Client::connect(server).await;
    let pane_id = client.open_pane().await;
    client.print_lines(pane_id, 200).await;
    client.wait_row("line-200").await;
    assert!(!client.screen.has_row("line-100"));
    (client, pane_id)
}

#[tokio::test]
async fn scrolling_back_shows_history_and_scrolling_forward_returns() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;

    client.scroll(pane_id, -50).await;
    client.wait_row("line-140").await;
    assert!(!client.screen.has_row("line-200"));
    // The screen is a window on consecutive lines.
    assert!(client.screen.has_row("line-139"));
    assert!(client.screen.has_row("line-141"));

    client.scroll(pane_id, 50).await;
    client.wait_row("line-200").await;
    assert!(!client.screen.has_row("line-140"));
}

#[tokio::test]
async fn to_bottom_returns_to_the_live_screen() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;
    client.scroll(pane_id, -100).await;
    client.wait_row("line-90").await;

    client.scroll_to_bottom(pane_id).await;
    client.wait_row("line-200").await;
    assert!(!client.screen.has_row("line-90"));
}

#[tokio::test]
async fn scrolling_stops_at_both_ends_of_history() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;

    client.scroll(pane_id, -1_000_000).await;
    client.wait_row("line-1").await;
    client.scroll(pane_id, -1_000_000).await;
    client.scroll(pane_id, 5).await;
    client.wait_row("line-30").await;

    client.scroll(pane_id, 1_000_000).await;
    client.wait_row("line-200").await;
    // The pane still works after all that.
    assert_eq!(client.run(pane_id, "true").await, 0);
}

#[tokio::test]
async fn typing_returns_the_view_to_the_live_screen() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;
    client.scroll(pane_id, -100).await;
    client.wait_row("line-90").await;

    client.press(pane_id, 'x').await;
    client.wait_row("line-200").await;
    assert!(!client.screen.has_row("line-90"));
}

#[tokio::test]
async fn the_scroll_position_is_shared_by_everyone_watching() {
    let server = Server::start(&[]);
    let (mut first, pane_id) = pane_with_history(&server).await;
    let mut second = Client::connect(&server).await;
    second.subscribe(pane_id).await;
    second.wait_row("line-200").await;

    first.scroll(pane_id, -100).await;
    first.wait_row("line-90").await;
    second.wait_row("line-90").await;
    assert!(!second.screen.has_row("line-200"));
}

#[tokio::test]
async fn a_client_joining_a_scrolled_pane_sees_the_scrolled_view() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;
    client.scroll(pane_id, -100).await;
    client.wait_row("line-90").await;
    drop(client);

    // The position belongs to the pane, not to the client that set it.
    let mut client = Client::connect(&server).await;
    client.subscribe(pane_id).await;
    client.wait_row("line-90").await;
    assert!(!client.screen.has_row("line-200"));
    client.scroll_to_bottom(pane_id).await;
    client.wait_row("line-200").await;
}

#[tokio::test]
async fn history_is_still_there_after_reconnecting() {
    let server = Server::start(&[]);
    let (client, pane_id) = pane_with_history(&server).await;
    drop(client);

    let mut client = Client::connect(&server).await;
    client.subscribe(pane_id).await;
    client.wait_row("line-200").await;
    client.scroll(pane_id, -1_000_000).await;
    client.wait_row("line-1").await;
}

#[tokio::test]
async fn history_survives_a_reload() {
    let server = Server::start(&[]);
    let (client, pane_id) = pane_with_history(&server).await;

    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    client.wait_row("line-200").await;
    client.scroll(pane_id, -1_000_000).await;
    client.wait_row("line-1").await;
    client.scroll(pane_id, 100).await;
    client.wait_row("line-110").await;
    client.scroll_to_bottom(pane_id).await;
    client.wait_row("line-200").await;
}

#[tokio::test]
async fn a_scrolled_view_stays_scrolled_through_a_reload() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;
    client.scroll(pane_id, -100).await;
    client.wait_row("line-90").await;

    let mut client = reload(&server, client).await;
    client.subscribe(pane_id).await;
    client.wait_row("line-90").await;
    assert!(!client.screen.has_row("line-200"));
}

#[tokio::test]
async fn output_arriving_while_scrolled_back_does_not_move_the_view() {
    let server = Server::start(&[]);
    let (mut client, pane_id) = pane_with_history(&server).await;
    client
        .type_line(pane_id, "sleep 1; echo \"new out\"\"put\"")
        .await;
    client.wait_screen("sleep 1").await;
    client.scroll(pane_id, -100).await;
    client.wait_row("line-90").await;

    tokio::time::sleep(Duration::from_millis(1500)).await;
    client.list_panes().await;
    // Still reading the same lines; the new output waits at the bottom.
    assert!(client.screen.has_row("line-90"));
    assert!(!client.screen.has_row("new output"));
    client.scroll_to_bottom(pane_id).await;
    client.wait_row("new output").await;
}
