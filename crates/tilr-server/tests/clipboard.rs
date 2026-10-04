//! Use cases: pasting through the shared clipboard.

mod common;

use common::*;

#[tokio::test]
async fn a_pasted_image_reaches_a_program_that_reads_the_clipboard() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    client.hello().await;
    let pane_id = client.open_pane().await;

    // Nothing was pasted yet: Claude Code finds no image.
    let check = script(&server, "check.sh", CLAUDE_CHECK_IMAGE);
    assert_ne!(client.run(pane_id, &check).await, 0);

    // Large enough to travel in several chunks.
    let data = image(2 * CHUNK + 12_345);
    client.put_clipboard(1, "image/png", &data).await;
    // The client does not wait for the ack: what it sends next is handled
    // after the upload, so the paste keystroke would find the image.
    assert_eq!(client.run(pane_id, &check).await, 0);
    let ack = client.clipboard_ack().await;
    assert!(ack.ok, "{}", ack.error);
    assert_eq!(ack.transfer_id, 1);

    let saved = server.dir.join("claude_cli_latest_screenshot.png");
    let save = script(&server, "save.sh", &claude_save_image(&saved));
    assert_eq!(client.run(pane_id, &save).await, 0);
    assert_eq!(std::fs::read(&saved).unwrap(), data);

    // The same through wl-paste alone.
    let listed = server.dir.join("types");
    let list = format!("wl-paste -l > '{}'", listed.display());
    assert_eq!(client.run(pane_id, &list).await, 0);
    assert_eq!(std::fs::read_to_string(&listed).unwrap(), "image/png\n");
    let pasted = server.dir.join("wl.png");
    let paste = format!("wl-paste --type image/png > '{}'", pasted.display());
    assert_eq!(client.run(pane_id, &paste).await, 0);
    assert_eq!(std::fs::read(&pasted).unwrap(), data);

    // An image is not text: a program asking for text gets nothing.
    assert_ne!(
        client.run(pane_id, "xclip -selection clipboard -o").await,
        0
    );
}

#[tokio::test]
async fn the_clipboard_is_shared_by_all_panes_and_replaced_by_the_next_paste() {
    let server = Server::start(&[]);
    let mut client = Client::connect(&server).await;
    let first = client.open_pane().await;
    client
        .put_clipboard(1, "text/plain", b"from the first paste")
        .await;
    assert!(client.clipboard_ack().await.ok);

    // A pane opened later, watched by another client, sees the same content.
    let mut other = Client::connect(&server).await;
    let second = other.open_pane().await;
    let out = server.dir.join("second.txt");
    let read = format!("xclip -selection clipboard -o > '{}'", out.display());
    assert_eq!(other.run(second, &read).await, 0);
    assert_eq!(std::fs::read(&out).unwrap(), b"from the first paste");

    other.put_clipboard(1, "text/plain", b"replaced").await;
    assert!(other.clipboard_ack().await.ok);
    let out = server.dir.join("first.txt");
    let read = format!("wl-paste --no-newline > '{}'", out.display());
    assert_eq!(client.run(first, &read).await, 0);
    assert_eq!(std::fs::read(&out).unwrap(), b"replaced");
}

#[tokio::test]
async fn an_upload_over_the_limit_is_refused_and_leaves_no_stale_content() {
    let server = Server::start(&["--clipboard-max", "64K"]);
    let mut client = Client::connect(&server).await;
    assert_eq!(client.hello().await.clipboard_max_bytes, 64 << 10);
    let pane_id = client.open_pane().await;
    let check = script(&server, "check.sh", CLAUDE_CHECK_IMAGE);

    client.put_clipboard(1, "image/png", &image(1000)).await;
    assert!(client.clipboard_ack().await.ok);
    assert_eq!(client.run(pane_id, &check).await, 0);

    // A gigabyte is announced: the server says no on the first chunk.
    client
        .send(Msg::ClipboardPut(ClipboardPut {
            transfer_id: 2,
            mime: "image/png".into(),
            total_size: 1 << 30,
            data: Bytes::from(image(CHUNK)),
            last: false,
        }))
        .await;
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert_eq!(ack.transfer_id, 2);
    assert!(ack.error.contains("too large: 1 GB"), "{}", ack.error);
    assert!(ack.error.contains("at most 64 KB"), "{}", ack.error);
    // The image pasted before must not come back in its place.
    assert_ne!(client.run(pane_id, &check).await, 0);

    // Announcing less than is sent does not get around the limit.
    client.put_clipboard(3, "image/png", &image(1000)).await;
    assert!(client.clipboard_ack().await.ok);
    for last in [false, true] {
        client
            .send(Msg::ClipboardPut(ClipboardPut {
                transfer_id: 4,
                mime: if last {
                    String::new()
                } else {
                    "image/png".into()
                },
                total_size: if last { 0 } else { 100 },
                data: Bytes::from(image(80)),
                last,
            }))
            .await;
    }
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert_eq!(ack.transfer_id, 4);
    assert_ne!(client.run(pane_id, &check).await, 0);
}

#[tokio::test]
async fn the_clipboard_can_be_turned_off() {
    let server = Server::start(&["--no-clipboard"]);
    let mut client = Client::connect(&server).await;
    let ack = client.hello().await;
    assert_eq!(ack.features, ["ping"]);
    assert_eq!(ack.clipboard_max_bytes, 0);

    client
        .put_clipboard(1, "image/png", &image(CHUNK + 1))
        .await;
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert!(ack.error.contains("turned off"), "{}", ack.error);

    // The panes are left alone: no tools of the server's on their PATH.
    let pane_id = client.open_pane().await;
    let out = server.dir.join("env.txt");
    let print = format!("echo \"$PATH|$TILR_CLIPBOARD_DIR\" > '{}'", out.display());
    assert_eq!(client.run(pane_id, &print).await, 0);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "/usr/bin:/bin|\n");
}

#[tokio::test]
async fn pasted_content_expires() {
    let server = Server::start(&["--clipboard-ttl", "1"]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    let check = script(&server, "check.sh", CLAUDE_CHECK_IMAGE);
    client.put_clipboard(1, "image/png", &image(100)).await;
    assert!(client.clipboard_ack().await.ok);
    assert_eq!(client.run(pane_id, &check).await, 0);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_ne!(client.run(pane_id, &check).await, 0);
}
