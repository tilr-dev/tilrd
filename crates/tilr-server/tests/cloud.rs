//! Use cases: a server linked to tilr Cloud. Cloud is played by the test,
//! and `cloudflared` by a script that notes how it was started.

mod common;

use common::*;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const MACHINE_TOKEN: &str = "tilr_m_test";

/// One control connection, seen from Cloud.
struct Control {
    ws: WebSocketStream<TcpStream>,
    /// The `Authorization` header the server connected with.
    authorization: String,
}

impl Control {
    async fn send(&mut self, message: Value) {
        self.ws
            .send(Message::text(message.to_string()))
            .await
            .unwrap();
    }

    /// The next message of the server that is not a keepalive.
    async fn next(&mut self) -> Value {
        loop {
            let message = tokio::time::timeout(TIMEOUT, self.ws.next())
                .await
                .expect("timed out waiting for the server")
                .expect("server closed the control connection")
                .unwrap();
            match message {
                Message::Text(text) if text.as_str() != "ping" => {
                    return serde_json::from_str(text.as_str()).unwrap();
                }
                _ => {}
            }
        }
    }

    /// Read until the server reports its tunnel as `state`.
    async fn tunnel_is(&mut self, state: &str) -> Value {
        loop {
            let message = self.next().await;
            if message["type"] == "tunnel_status" && message["state"] == state {
                return message;
            }
        }
    }

    /// Take the server's hello and answer it.
    async fn greet(&mut self) -> Value {
        let hello = self.next().await;
        assert_eq!(hello["type"], "hello");
        self.send(json!({ "type": "hello_ack", "protocol": 1, "machine_id": "m1" }))
            .await;
        hello
    }
}

/// A Cloud that accepts control connections and hands them to the test.
async fn cloud() -> (String, mpsc::UnboundedReceiver<Control>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (connected, connections) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut authorization = String::new();
            let mut path = String::new();
            #[expect(
                clippy::result_large_err,
                reason = "the handshake callback signature is fixed by tungstenite"
            )]
            let note = |request: &Request, response: Response| {
                path = request.uri().path().to_owned();
                authorization = request
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                Ok(response)
            };
            let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, note).await else {
                continue;
            };
            assert_eq!(path, "/v1/machine/connect");
            if connected.send(Control { ws, authorization }).is_err() {
                break;
            }
        }
    });
    (url, connections)
}

/// A server linked to the Cloud at `url`, with the stand-in `cloudflared`.
fn linked_server(url: &str, args: &[&str]) -> Server {
    use std::os::unix::fs::PermissionsExt;

    let dir = Server::scratch_dir();
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(
        dir.join("state/cloud.json"),
        json!({ "url": url, "machine_id": "m1", "token": MACHINE_TOKEN }).to_string(),
    )
    .unwrap();
    let cloudflared = dir.join("cloudflared");
    std::fs::write(
        &cloudflared,
        format!(
            "#!/bin/sh\n\
             cd {dir}\n\
             echo \"$@\" > cloudflared.args\n\
             cat \"$3\" > cloudflared.config\n\
             echo $$ > cloudflared.pid\n\
             echo 'INF Registered tunnel connection connIndex=0' >&2\n\
             exec sleep 600\n",
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cloudflared, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut all = vec!["--cloudflared", cloudflared.to_str().unwrap()];
    all.extend(args);
    Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilr-server")), &all)
}

fn tunnel(hostname: &str) -> Value {
    json!({
        "type": "tunnel_open",
        "tunnel": {
            "id": "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4",
            "account_tag": "8e0530af975a33f2f739127f75084531",
            "secret": "c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0c2VjcmV0cw==",
            "hostname": hostname,
        },
    })
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let started = std::time::Instant::now();
    while !done() {
        assert!(
            started.elapsed() < TIMEOUT,
            "timed out waiting until {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_linked_server_gets_its_tunnel_from_cloud() {
    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &[]);
    let mut control = connections.recv().await.unwrap();
    assert_eq!(control.authorization, format!("Bearer {MACHINE_TOKEN}"));

    let hello = control.greet().await;
    assert_eq!(hello["protocol"], 1);
    assert_eq!(hello["auth"], true);
    assert_eq!(hello["platform"], env!("TILR_PLATFORM"));
    assert_eq!(control.next().await["state"], "down");

    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;

    // cloudflared was pointed at this server's port and nowhere else.
    let read = |name: &str| std::fs::read_to_string(server.dir.join(name)).unwrap();
    assert!(read("cloudflared.args").starts_with("tunnel --config "));
    let config = read("cloudflared.config");
    let port = server.addr.strip_prefix("ws://").unwrap();
    assert!(
        config.contains("tunnel: 6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4\n"),
        "{config}"
    );
    assert!(
        config.contains(&format!(
            "ingress:\n  - hostname: abc234.tilr.test\n    service: http://{port}\n  - service: http_status:404\n"
        )),
        "{config}"
    );
    let credentials = server.dir.join("runtime/cloudflared/credentials.json");
    let secret: Value =
        serde_json::from_str(&std::fs::read_to_string(credentials).unwrap()).unwrap();
    assert_eq!(secret["AccountTag"], "8e0530af975a33f2f739127f75084531");

    // Taken away: cloudflared is stopped and its secret removed.
    let pid = read("cloudflared.pid").trim().to_owned();
    assert!(alive(&pid));
    control
        .send(json!({ "type": "tunnel_close", "reason": "test" }))
        .await;
    control.tunnel_is("down").await;
    assert!(!alive(&pid));
    assert!(!server.dir.join("runtime/cloudflared").exists());
}

#[tokio::test]
async fn a_server_without_authentication_takes_no_tunnel() {
    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &["--no-auth"]);
    let mut control = connections.recv().await.unwrap();
    assert_eq!(control.greet().await["auth"], false);

    control.send(tunnel("abc234.tilr.test")).await;
    let refused = control.tunnel_is("failed").await;
    assert_eq!(refused["detail"], "the server runs without authentication");
    assert!(!server.dir.join("cloudflared.pid").exists());
}

#[tokio::test]
async fn a_tunnel_that_would_lead_elsewhere_is_refused() {
    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &[]);
    let mut control = connections.recv().await.unwrap();
    control.greet().await;

    control
        .send(tunnel("a.test\n    service: ssh://localhost:22"))
        .await;
    control.tunnel_is("failed").await;
    assert!(!server.dir.join("cloudflared.pid").exists());
}

#[tokio::test]
async fn a_reload_replaces_cloudflared_with_one_of_the_new_server() {
    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &[]);
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let pid_file = server.dir.join("cloudflared.pid");
    let before = std::fs::read_to_string(&pid_file).unwrap();

    server.signal("-USR1");
    // The server that took over connects again and is given the tunnel again.
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;

    let after = std::fs::read_to_string(&pid_file).unwrap();
    assert_ne!(before, after);
    until("the old cloudflared is gone", || !alive(before.trim())).await;
    assert!(alive(after.trim()));
}

#[tokio::test]
async fn a_server_linked_while_it_runs_connects_on_its_own() {
    let (url, mut connections) = cloud().await;
    let server = Server::start(&[]);
    std::fs::write(
        server.dir.join("state/cloud.json"),
        json!({ "url": url, "machine_id": "m1", "token": MACHINE_TOKEN }).to_string(),
    )
    .unwrap();
    let mut control = tokio::time::timeout(TIMEOUT, connections.recv())
        .await
        .expect("the server did not connect")
        .unwrap();
    control.greet().await;

    // Unlinked: the connection is given up.
    std::fs::remove_file(server.dir.join("state/cloud.json")).unwrap();
    loop {
        match tokio::time::timeout(TIMEOUT, control.ws.next())
            .await
            .unwrap()
        {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
            Some(Ok(_)) => {}
        }
    }
}
