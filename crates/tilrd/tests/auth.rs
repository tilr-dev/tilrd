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
        .env_remove("RUST_LOG")
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
async fn an_existing_state_directory_is_made_private() {
    use std::os::unix::fs::PermissionsExt;

    // Left 0755 by hand (or an older version), with an id and a token in it.
    let dir = Server::scratch_dir();
    let state = dir.join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let token = "ab".repeat(32);
    let id = "6f1c1d2e-3b4a-4c5d-8e6f-7a8b9c0d1e2f";
    std::fs::write(state.join("token"), format!("{token}\n")).unwrap();
    std::fs::set_permissions(state.join("token"), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(state.join("server-id"), format!("{id}\n")).unwrap();

    let server = Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), &[]);
    assert_eq!(
        std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(server.token(), token, "the same token");
    assert_eq!(
        refusal(&server, &token).await,
        None,
        "and it lets the client in"
    );
    assert_eq!(
        std::fs::read_to_string(state.join("server-id"))
            .unwrap()
            .trim(),
        id,
        "the same id"
    );
}

#[tokio::test]
async fn a_server_started_without_authentication_asks_for_no_token() {
    let server = Server::start(&["--no-auth"]);
    assert_eq!(server.token(), "");
    assert_eq!(refusal(&server, "").await, None);
    server.wait_logged("no authentication").await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello() {
    // No client timeout: only the Hello deadline lets the silent one go.
    let dir = Server::scratch_dir();
    let clock = dir.join("clock");
    const START_US: u64 = 1_700_000_000_000_000;
    set_test_time(&clock, START_US);
    let server = Server::start_with_env(
        dir,
        &["--client-timeout", "0"],
        &[("TILR_TEST_TIME_FILE", clock.to_str().unwrap())],
    );
    let url = format!("/metrics?token={}", server.token());
    let (_, text) = http_get(&server, &url, &[]).await;
    for reason in ["superseded", "too_slow", "silent", "no_pong", "no_hello"] {
        assert_eq!(
            sample(
                &text,
                &format!("tilrd_client_drops_total{{reason=\"{reason}\"}}")
            ),
            Some(0.0),
            "{text}"
        );
    }
    let mut silent = Client::connect_silent(&server).await;
    let silent_deadline = hello_deadline(&silent);
    server
        .wait_logged(&format!("test deadline ready: {silent_deadline}"))
        .await;
    let mut greets = Client::connect_silent(&server).await;
    greets.hello().await;
    let greeted_deadline = hello_deadline(&greets);
    server
        .wait_logged(&format!("test deadline ready: {greeted_deadline}"))
        .await;
    set_test_time(&clock, START_US + 9_999_999);
    signal_before_deadline(&clock, &silent_deadline).await;
    set_test_time(&clock, START_US + 10_000_000);
    signal_deadline(&clock, &silent_deadline).await;
    signal_deadline(&clock, &greeted_deadline).await;

    // Reading answers the server's pings, should it send any.
    let close = loop {
        match tokio::time::timeout(TIMEOUT, silent.ws.next()).await {
            Ok(Some(Ok(Message::Close(frame)))) => break frame,
            Ok(Some(Ok(_))) => {}
            other => panic!("connection ended without a close frame: {other:?}"),
        }
    };
    let frame = close.expect("a close frame with a code");
    assert_eq!(u16::from(frame.code), 4002);
    assert_eq!(frame.reason.as_str(), "timeout");
    server.wait_logged("client sent no Hello in 10s").await;

    // Both test-clock deadlines were processed; the greeted client stays.
    assert!(greets.list_panes().await.is_empty());

    // Only the silent one is counted, and as no_hello.
    let (status, text) = http_get(&server, &url, &[]).await;
    assert_eq!(status, 200);
    assert_eq!(
        sample(&text, "tilrd_client_drops_total{reason=\"no_hello\"}"),
        Some(1.0),
        "{text}"
    );
    for reason in ["superseded", "too_slow", "silent", "no_pong"] {
        assert_eq!(
            sample(
                &text,
                &format!("tilrd_client_drops_total{{reason=\"{reason}\"}}")
            ),
            Some(0.0),
            "{text}"
        );
    }
}
