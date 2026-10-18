//! Use cases: who may use a server.

mod common;

use common::*;

/// What the server answers a hello carrying `token` with: the refusal, or
/// `None` if it was let in.
async fn refusal(server: &Server, token: &str) -> Option<String> {
    let mut client = Client::connect_silent(server).await;
    client.token = token.to_owned();
    client.hello_as(PROTOCOL_VERSION).await;
    match client.next().await {
        Some(server_message::Msg::Error(error)) => {
            assert!(client.next().await.is_none(), "connection left open");
            Some(error.message)
        }
        Some(server_message::Msg::HelloAck(_)) => None,
        other => panic!("unexpected answer to hello: {other:?}"),
    }
}

#[tokio::test]
async fn a_client_needs_the_access_token() {
    let server = Server::start(&[]);
    let token = server.token();
    assert_eq!(token.len(), 64);

    assert_eq!(
        refusal(&server, "").await.as_deref(),
        Some("this server asks for its access token")
    );
    let mut wrong = token.clone();
    wrong.replace_range(..1, if token.starts_with('0') { "1" } else { "0" });
    assert_eq!(
        refusal(&server, &wrong).await.as_deref(),
        Some("wrong access token")
    );
    assert_eq!(refusal(&server, &token).await, None);

    // A client turned away got nothing done.
    let mut client = Client::connect(&server).await;
    assert!(client.list_panes().await.is_empty());
}

#[tokio::test]
async fn the_token_is_kept_for_the_user_only_and_across_restarts() {
    use std::os::unix::fs::PermissionsExt;

    let mut server = Server::start(&[]);
    let token = server.token();
    let file = server.dir.join("state/token");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // In a directory of the user's alone, made by the server.
    assert_eq!(
        std::fs::metadata(server.dir.join("state"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    // `tilrd token` prints the same one.
    let printed = Command::new(env!("CARGO_BIN_EXE_tilrd"))
        .arg("token")
        .env("TILR_STATE_DIR", server.dir.join("state"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(printed.stdout).unwrap().trim(), token);

    let dir = server.dir.clone();
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    // Dropping the server would remove its directory.
    std::mem::forget(server);
    let again = Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), &[]);
    assert_eq!(again.token(), token);
}

#[tokio::test]
async fn a_server_started_without_authentication_asks_for_no_token() {
    let server = Server::start(&["--no-auth"]);
    assert_eq!(server.token(), "");
    assert_eq!(refusal(&server, "").await, None);
    assert!(server.logged("no authentication"));
}

#[tokio::test]
async fn a_connection_that_never_says_hello_is_closed_after_10_s() {
    // No client timeout: only the Hello deadline lets the silent one go.
    let server = Server::start(&["--client-timeout", "0"]);
    let started = std::time::Instant::now();
    let mut silent = Client::connect_silent(&server).await;
    let mut greets = Client::connect_silent(&server).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    greets.hello().await;

    // Reading answers the server's pings, should it send any.
    let close = loop {
        match tokio::time::timeout(TIMEOUT, silent.ws.next()).await {
            Ok(Some(Ok(Message::Close(frame)))) => break frame,
            Ok(Some(Ok(_))) => {}
            other => panic!("connection ended without a close frame: {other:?}"),
        }
    };
    let waited = started.elapsed();
    let frame = close.expect("a close frame with a code");
    assert_eq!(u16::from(frame.code), 4002);
    assert_eq!(frame.reason.as_str(), "timeout");
    assert!(
        (Duration::from_secs(9)..Duration::from_secs(13)).contains(&waited),
        "closed after {waited:?}"
    );
    assert!(server.logged("client sent no Hello in 10s"));

    // The one that said Hello after a second stays.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(greets.list_panes().await.is_empty());
}
