//! Clear the server-owned screen without sending input to its program.
mod common;

use common::*;
use tilr_proto::ClearPane;

async fn clear(client: &mut Client, pane: PaneId, history: bool) {
    client
        .send(Msg::ClearPane(ClearPane {
            pane_id: pane.to_string(),
            history,
        }))
        .await;
}

async fn blank_frame(client: &mut Client, pane: PaneId) -> Frame {
    let frame = client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame)
                if frame.pane_id == pane.to_string()
                    && frame.full
                    && frame.changed.iter().all(|row| row.runs.is_empty()) =>
            {
                Some(frame.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(frame.changed.len(), frame.rows as usize);
    let cursor = frame.cursor.as_ref().unwrap();
    assert_eq!((cursor.x, cursor.y), (0, 0));
    assert!(client.screen.text().trim().is_empty());
    frame
}

async fn history(server: &Server) -> (Client, PaneId) {
    let mut client = Client::connect(server).await;
    let pane = client.open_pane().await;
    // The title is emitted after all history, and read prevents another prompt.
    client.type_line(pane, "i=1; while [ $i -le 200 ]; do echo line-$i; i=$((i+1)); done; printf '\\033]2;history-ready\\007'; read _").await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame)
                if frame.title.as_deref() == Some("history-ready") =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(client.screen.has_row("line-200"));
    (client, pane)
}

#[tokio::test]
async fn clearing_history_reaches_all_viewers_and_survives_reconnecting() {
    let server = Server::start(&[]);
    let (mut first, pane) = history(&server).await;
    let mut second = Client::connect(&server).await;
    second.subscribe(pane).await;
    second.wait_row("line-200").await;
    first.scroll(pane, -100).await;
    first.wait_row("line-100").await;
    second.wait_row("line-100").await;

    clear(&mut first, pane, true).await;
    assert_eq!(blank_frame(&mut first, pane).await.viewport_top, 0);
    assert_eq!(blank_frame(&mut second, pane).await.viewport_top, 0);
    // Even clearing an already empty screen must broadcast a full frame.
    clear(&mut first, pane, true).await;
    blank_frame(&mut first, pane).await;
    blank_frame(&mut second, pane).await;

    first.scroll(pane, -1_000_000).await;
    first.subscribe(pane).await;
    assert_eq!(blank_frame(&mut first, pane).await.viewport_top, 0);
    drop(first);
    let mut reconnected = Client::connect(&server).await;
    reconnected.subscribe(pane).await;
    assert_eq!(blank_frame(&mut reconnected, pane).await.viewport_top, 0);
}

#[tokio::test]
async fn clearing_only_the_screen_keeps_history() {
    let server = Server::start(&[]);
    let (mut client, pane) = history(&server).await;
    let live_top = client.frame.as_ref().unwrap().viewport_top;
    assert!(live_top > 0);
    client.scroll(pane, -100).await;
    client.wait_row("line-100").await;
    clear(&mut client, pane, false).await;
    assert_eq!(blank_frame(&mut client, pane).await.viewport_top, live_top);
    client.scroll(pane, -100).await;
    client.wait_row("line-100").await;
    client.scroll_to_bottom(pane).await;
    client.subscribe(pane).await;
    assert_eq!(blank_frame(&mut client, pane).await.viewport_top, live_top);
}

#[tokio::test]
async fn clearing_does_not_write_to_the_program_and_output_starts_at_home() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane = client.open_pane().await;
    let (fifo, _release) = activity_fifo(&server, "hold-program");
    // Read exactly two bytes with echo and canonical buffering disabled.
    // Any pty write from ClearPane would be consumed before our GO marker.
    client.type_line(pane, &format!("stty -echo -icanon min 1 time 0; printf '\\033]2;reading\\007'; bytes=$(dd bs=1 count=2 2>/dev/null); printf 'got:<%s>' \"$bytes\"; read _ < '{}'", fifo.display())).await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) if frame.title.as_deref() == Some("reading") => {
                Some(())
            }
            _ => None,
        })
        .await;
    clear(&mut client, pane, true).await;
    blank_frame(&mut client, pane).await;
    client
        .send(Msg::Raw(RawInput {
            pane_id: pane.to_string(),
            data: Bytes::from_static(b"GO"),
        }))
        .await;
    client.wait_row("got:<GO>").await;
    assert_eq!(client.screen.text().lines().next().unwrap(), "got:<GO>");
    assert_eq!(client.list_panes().await.len(), 1);
}

#[tokio::test]
async fn clearing_an_unknown_pane_is_an_error() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let pane = PaneId::new_v4();
    clear(&mut client, pane, true).await;
    assert_eq!(client.error().await, format!("no such pane: {pane}"));
    assert!(client.list_panes().await.is_empty());
}
