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
    linked_server_in(
        Server::scratch_dir(),
        url,
        args,
        "cloudflared",
        "exec sleep 600\n",
    )
}

/// A server linked to the Cloud at `url` in `dir`, with a stand-in
/// `cloudflared` there named `name` that, once it said it registered, runs
/// `then`.
fn linked_server_in(dir: PathBuf, url: &str, args: &[&str], name: &str, then: &str) -> Server {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(
        dir.join("state/cloud.json"),
        json!({ "url": url, "machine_id": "m1", "token": MACHINE_TOKEN }).to_string(),
    )
    .unwrap();
    let cloudflared = dir.join(name);
    std::fs::write(
        &cloudflared,
        format!(
            "#!/bin/sh\n\
             cd {dir}\n\
             echo \"$@\" > cloudflared.args\n\
             cat \"$3\" > cloudflared.config\n\
             echo $$ > cloudflared.pid\n\
             echo 'INF Registered tunnel connection connIndex=0' >&2\n\
             {then}",
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cloudflared, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut all = vec!["--cloudflared", cloudflared.to_str().unwrap()];
    all.extend(args);
    Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilrd")), &all)
}

fn tunnel(hostname: &str) -> Value {
    json!({
        "type": "tunnel_open",
        "tunnel": {
            "id": "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4",
            "account_tag": "0123456789abcdef0123456789abcdef",
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
    assert_eq!(secret["AccountTag"], "0123456789abcdef0123456789abcdef");

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

/// The process group of `pid`, as `ps` shows it.
fn process_group(pid: &str) -> String {
    let output = Command::new("ps")
        .args(["-o", "pgid=", "-p", pid])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[tokio::test]
async fn cloudflared_is_not_in_the_servers_process_group() {
    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &[]);
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let pid = std::fs::read_to_string(server.dir.join("cloudflared.pid")).unwrap();
    let pid = pid.trim();
    // A group of its own, which a signal to the server's group (a terminal
    // hanging up, Ctrl-C) does not reach.
    assert_eq!(process_group(pid), pid);
    assert_ne!(
        process_group(pid),
        process_group(&server.child.id().to_string())
    );
}

/// A stand-in that ignores SIGTERM and keeps its command line (the
/// configuration among its arguments) while it runs.
const IGNORES_SIGTERM: &str = "trap '' TERM\nwhile :; do sleep 1; done\n";

#[tokio::test]
async fn a_cloudflared_left_by_a_killed_server_is_stopped() {
    let (url, mut connections) = cloud().await;
    let mut killed = linked_server_in(
        Server::scratch_dir(),
        &url,
        &[],
        "cloudflared",
        IGNORES_SIGTERM,
    );
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let pid = std::fs::read_to_string(killed.dir.join("cloudflared.pid")).unwrap();
    let pid = pid.trim().to_owned();
    assert!(
        killed.dir.join("runtime/cloudflared/pid").exists(),
        "the server notes the pid"
    );

    // No cleanup runs: a SIGKILL. Even with the parent-death signal (Linux)
    // this one is still there, as it ignores SIGTERM.
    killed.child.kill().unwrap();
    killed.child.wait().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(alive(&pid), "left behind");

    // The next server on the same runtime directory stops it.
    let next = linked_server_in(
        killed.dir.clone(),
        &url,
        &[],
        "cloudflared-next",
        "exec sleep 600\n",
    );
    until("the leftover is stopped", || {
        next.logged(&format!(
            "stopped a cloudflared left by an earlier server (pid {pid})"
        ))
    })
    .await;
    until("the leftover is gone", || !alive(&pid)).await;
    // And the next tunnel is its own.
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let started = std::fs::read_to_string(next.dir.join("cloudflared.pid")).unwrap();
    assert_ne!(started.trim(), pid);
}

#[tokio::test]
async fn a_cloudflared_ignoring_sigterm_does_not_hold_back_a_reload() {
    let (url, mut connections) = cloud().await;
    let server = linked_server_in(
        Server::scratch_dir(),
        &url,
        &[],
        "cloudflared",
        "trap '' TERM\nexec sleep 600\n",
    );
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let pid_file = server.dir.join("cloudflared.pid");
    let before = std::fs::read_to_string(&pid_file).unwrap();

    let reloaded = std::time::Instant::now();
    server.signal("-USR1");
    let mut control = connections.recv().await.unwrap();
    // Killed once the SIGTERM went unheeded for 3 s, then the exec.
    assert!(
        reloaded.elapsed() >= Duration::from_secs(2),
        "{:?}",
        reloaded.elapsed()
    );
    assert!(
        !alive(before.trim()),
        "the old cloudflared is gone before the exec"
    );
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let after = std::fs::read_to_string(&pid_file).unwrap();
    assert_ne!(before, after);
    assert!(alive(after.trim()));
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_panic_starting_cloudflared_does_not_hold_up_the_next_reload() {
    // The start of a child panics while the reload signals are let in on
    // that thread; a reload waits for no child being started.
    let (url, mut connections) = cloud().await;
    let dir = Server::scratch_dir();
    let failpoints = dir.join("failpoints");
    std::fs::write(&failpoints, "spawn-unblocked=panic").unwrap();
    let server = linked_server_in(dir, &url, &[], "cloudflared", "exec sleep 600\n");
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    until("starting cloudflared panics", || {
        server.logged("failpoint spawn-unblocked")
    })
    .await;
    std::fs::write(&failpoints, "").unwrap();

    server.signal("-USR1");
    until("the server reloads", || {
        server.logged("resumed after reload")
    })
    .await;
    assert!(!server.logged("reload failed"));
    // The successor starts cloudflared as usual.
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
}
