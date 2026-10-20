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
    assert_eq!(ack.features, ["files", "host-events", "agent", "exec"]);
    assert_eq!(ack.clipboard_max_bytes, 0);

    client
        .put_clipboard(1, "image/png", &image(CHUNK + 1))
        .await;
    let ack = client.clipboard_ack().await;
    assert!(!ack.ok);
    assert!(ack.error.contains("turned off"), "{}", ack.error);

    // The panes are left alone: no clipboard tools of the server's on their
    // PATH (only its `tilr`).
    let pane_id = client.open_pane().await;
    let out = server.dir.join("env.txt");
    let print = format!(
        "echo \"$(command -v xclip wl-paste wl-copy)|$TILR_CLIPBOARD_DIR|$TILR_CLIPBOARD_MAX\" > '{}'",
        out.display()
    );
    assert_eq!(client.run(pane_id, &print).await, 0);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "||\n");
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

/// `printf` of an OSC 52 copy of `base64` to the clipboard.
fn osc_52(base64: &str) -> String {
    format!("printf '\\033]52;c;{base64}\\007'")
}

/// A client that takes the copies made in the panes it watches.
async fn copying_client(server: &Server) -> Client {
    let mut client = Client::connect_silent(server).await;
    client.hello_copying().await;
    client
}

/// What `command` run in `pane_id` writes to its stdout.
async fn output(server: &Server, client: &mut Client, pane_id: PaneId, command: &str) -> Vec<u8> {
    let out = server.dir.join("output");
    let _ = std::fs::remove_file(&out);
    let redirected = format!("{command} > '{}'", out.display());
    assert_eq!(client.run(pane_id, &redirected).await, 0, "{command}");
    std::fs::read(&out).unwrap()
}

#[tokio::test]
async fn osc_52_in_a_pane_reaches_the_watching_client() {
    let server = Server::start(&[]);
    let mut a = copying_client(&server).await;
    let pane_id = a.open_pane().await;
    // B watches the pane but does not take copies; C takes them but
    // watches another pane.
    let mut b = Client::connect(&server).await;
    b.subscribe(pane_id).await;
    let mut c = copying_client(&server).await;
    let other = c.open_pane().await;

    assert_eq!(a.run(pane_id, &osc_52("aGk=")).await, 0);
    let copy = a.copied().await;
    assert_eq!(copy.pane_id, pane_id.to_string());
    assert_eq!(copy.mime, "text/plain;charset=utf-8");
    assert_eq!(copy.data, b"hi");
    assert_eq!((copy.chunks, copy.total_size), (1, 2));
    assert!(copy.abandoned.is_empty());

    // It is the shared clipboard: another pane pastes it.
    assert_eq!(
        output(&server, &mut c, other, "xclip -selection clipboard -o").await,
        b"hi"
    );
    // The first copy C gets is the one made in its own pane.
    assert_eq!(c.run(other, &osc_52("dGhlcmU=")).await, 0);
    let copy = c.copied().await;
    assert_eq!(
        (copy.pane_id, copy.data),
        (other.to_string(), b"there".to_vec())
    );
    // B got none: anything queued for it before would come before this.
    b.sync_bulk(&server).await;
    assert!(b.copies.is_empty(), "{:?}", b.copies);
}

#[tokio::test]
async fn xclip_in_a_pane_reaches_the_watching_client() {
    let server = Server::start(&[]);
    let mut a = copying_client(&server).await;
    let pane_id = a.open_pane().await;
    let data = image(100_000);
    let pic = server.dir.join("pic.png");
    std::fs::write(&pic, &data).unwrap();

    let copy_image = format!(
        "xclip -selection clipboard -t image/png -i < '{}'",
        pic.display()
    );
    assert_eq!(a.run(pane_id, &copy_image).await, 0);
    let copy = a.copied().await;
    assert_eq!(copy.pane_id, pane_id.to_string());
    assert_eq!(copy.mime, "image/png");
    assert!(copy.data == data, "the image came changed");

    // Another pane, watched by another client, pastes it.
    let mut other = Client::connect(&server).await;
    let second = other.open_pane().await;
    let pasted = output(&server, &mut other, second, "wl-paste -t image/png").await;
    assert!(pasted == data, "the pasted image is not the copied one");

    // wl-copy copies its arguments, or stdin.
    assert_eq!(a.run(pane_id, "wl-copy copied with wl-copy").await, 0);
    let copy = a.copied().await;
    assert_eq!(copy.mime, "text/plain;charset=utf-8");
    assert_eq!(copy.data, b"copied with wl-copy");
    assert_eq!(a.run(pane_id, "echo piped | wl-copy").await, 0);
    assert_eq!(a.copied().await.data, b"piped\n");
}

#[tokio::test]
async fn a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it() {
    let server = Server::start(&[]);
    let mut client = copying_client(&server).await;
    let pane_id = client.open_pane().await;

    let data = image(1 << 20);
    let big = server.dir.join("big");
    std::fs::write(&big, &data).unwrap();
    let copy_big = format!("xclip -selection clipboard -i < '{}'", big.display());
    assert_eq!(client.run(pane_id, &copy_big).await, 0);
    let copy = client.copied().await;
    assert_eq!(copy.chunks, 4);
    assert_eq!(copy.total_size, 1 << 20);
    assert!(copy.data == data, "the copy came changed");

    // 8 MiB cannot all get to a client that reads nothing (the bulk lane
    // holds 4 MiB, and 1 MiB more goes out before the client's pongs): the
    // second copy comes while the first is still being sent.
    let huge = server.dir.join("huge");
    std::fs::write(&huge, image(8 << 20)).unwrap();
    let done = server.dir.join("done");
    let made = Command::new("mkfifo").arg(&done).status().unwrap();
    assert!(made.success());
    client
        .type_line(
            pane_id,
            &format!(
                "xclip -selection clipboard -i < '{}' && echo second | xclip -selection clipboard -i; echo copied > '{}'",
                huge.display(),
                done.display()
            ),
        )
        .await;
    // Returns once the shell has made both copies, without reading from
    // the server meanwhile.
    let said = tokio::task::spawn_blocking(move || std::fs::read_to_string(done))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(said, "copied\n");

    let copy = client.copied().await;
    assert_eq!(copy.data, b"second\n");
    assert_eq!(copy.abandoned.len(), 1, "the first copy must be cut short");
    assert!(copy.abandoned[0] < copy.transfer_id);
    // And nothing more of it comes: a first copy still going on would be
    // waiting for room in the lane ahead of this read.
    client.sync_bulk(&server).await;
    assert!(
        client.copies.is_empty(),
        "{} chunks of the first copy came after the second",
        client.copies.len()
    );
}

#[tokio::test]
async fn nothing_is_copied_to_clients_with_the_clipboard_turned_off() {
    let server = Server::start(&["--no-clipboard"]);
    let mut client = copying_client(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, &osc_52("aGk=")).await, 0);
    // Anything queued for the client before would come before this.
    client.sync_bulk(&server).await;
    assert!(client.copies.is_empty(), "{:?}", client.copies);
}

#[tokio::test]
async fn a_copy_over_the_limit_is_refused() {
    let server = Server::start(&["--clipboard-max", "1K"]);
    let mut client = copying_client(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(
        output(
            &server,
            &mut client,
            pane_id,
            "printf %s \"$TILR_CLIPBOARD_MAX\""
        )
        .await,
        b"1024"
    );
    assert_eq!(
        client
            .run(pane_id, "echo before | xclip -selection clipboard -i")
            .await,
        0
    );
    assert_eq!(client.copied().await.data, b"before\n");

    let large = server.dir.join("large");
    std::fs::write(&large, vec![b'x'; 2048]).unwrap();
    let errors = server.dir.join("errors");
    let copy_large = format!(
        "xclip -selection clipboard -i < '{}' 2> '{}'",
        large.display(),
        errors.display()
    );
    assert_ne!(client.run(pane_id, &copy_large).await, 0);
    let error = std::fs::read_to_string(&errors).unwrap();
    assert_eq!(
        error,
        "xclip: too large: 2 KB, the clipboard takes at most 1 KB\n"
    );
    // The clipboard holds what it held.
    assert_eq!(
        output(
            &server,
            &mut client,
            pane_id,
            "xclip -selection clipboard -o"
        )
        .await,
        b"before\n"
    );

    // An OSC 52 copy of 2 KiB (2048 zero bytes) is dropped.
    let osc = server.dir.join("osc");
    let base64 = format!("{}AA==", "A".repeat(2728));
    std::fs::write(&osc, format!("\x1b]52;c;{base64}\x07")).unwrap();
    assert_eq!(
        client
            .run(pane_id, &format!("cat '{}'", osc.display()))
            .await,
        0
    );
    server
        .wait_logged("clipboard: a copy over the limit was dropped")
        .await;
    // The first copy the client gets after it is the next one.
    assert_eq!(client.run(pane_id, "wl-copy after").await, 0);
    let copy = client.copied().await;
    assert_eq!(copy.data, b"after");
    assert!(copy.abandoned.is_empty());
}

#[tokio::test]
async fn a_copy_nobody_takes_stays_in_the_shared_buffer() {
    let server = Server::start(&["--clipboard-ttl", "3600"]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    // A real xclip, later on PATH, notes that it ran.
    let fake = server.dir.join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    let ran = server.dir.join("real-xclip-ran");
    std::fs::write(
        fake.join("xclip"),
        format!("#!/bin/sh\ntouch '{}'\ncat > /dev/null\n", ran.display()),
    )
    .unwrap();
    let made = Command::new("chmod")
        .arg("+x")
        .arg(fake.join("xclip"))
        .status();
    assert!(made.unwrap().success());
    let path = format!("PATH=\"$PATH:{}\"", fake.display());
    assert_eq!(client.run(pane_id, &path).await, 0);

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert_eq!(
        client
            .run(pane_id, "echo hi | xclip -selection clipboard -i")
            .await,
        0
    );
    assert!(!ran.exists(), "the real xclip was run");
    assert_eq!(
        output(
            &server,
            &mut client,
            pane_id,
            "xclip -selection clipboard -o"
        )
        .await,
        b"hi\n"
    );
    // Kept until --clipboard-ttl: the store's second line is when it
    // expires, in Unix milliseconds.
    let stored = output(
        &server,
        &mut client,
        pane_id,
        "sed -n 2p \"$TILR_CLIPBOARD_DIR/content\"",
    )
    .await;
    let expires: u64 = String::from_utf8(stored).unwrap().trim().parse().unwrap();
    assert!(expires >= before + 3_600_000, "{expires}");

    // A client that comes later is not sent it: the first copy it gets is
    // the next one.
    let mut later = copying_client(&server).await;
    later.subscribe(pane_id).await;
    assert_eq!(
        client
            .run(pane_id, "echo later | xclip -selection clipboard -i")
            .await,
        0
    );
    let copy = later.copied().await;
    assert_eq!(copy.data, b"later\n");
    assert!(copy.abandoned.is_empty());
}
