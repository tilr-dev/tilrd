//! Use cases: a server linked to tilr Cloud. Cloud is played by the test,
//! and `cloudflared` by a script that notes how it was started.

mod common;

use common::*;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const MACHINE_TOKEN: &str = "tilr_m_test";

/// One control connection, seen from Cloud.
struct Control {
    ws: WebSocketStream<TcpStream>,
    /// The `Authorization` header the server connected with.
    authorization: String,
}

struct CloudRequest {
    method: String,
    path: String,
    authorization: String,
    body: Value,
    answer: oneshot::Sender<(u16, Value)>,
}

impl CloudRequest {
    fn respond(self, status: u16, body: Value) {
        self.answer.send((status, body)).unwrap();
    }
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
    let (url, connections, _requests) = cloud_with_http().await;
    (url, connections)
}

async fn cloud_with_http() -> (
    String,
    mpsc::UnboundedReceiver<Control>,
    mpsc::UnboundedReceiver<CloudRequest>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (connected, connections) = mpsc::unbounded_channel();
    let (requested, requests) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut first = [0];
            if stream.peek(&mut first).await.is_err() {
                continue;
            }
            if first[0] != b'G' {
                let requested = requested.clone();
                tokio::spawn(async move { answer_http(stream, requested).await });
                continue;
            }
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
    (url, connections, requests)
}

async fn answer_http(mut stream: TcpStream, requested: mpsc::UnboundedSender<CloudRequest>) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 1024];
        let size = stream.read(&mut chunk).await.unwrap();
        assert!(size > 0, "Cloud HTTP request ended before headers");
        bytes.extend_from_slice(&chunk[..size]);
        if let Some(pos) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap().to_owned();
    let path = parts.next().unwrap().to_owned();
    let mut content_length = 0;
    let mut authorization = String::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap();
            } else if name.eq_ignore_ascii_case("authorization") {
                authorization = value.trim().to_owned();
            }
        }
    }
    while bytes.len() - header_end < content_length {
        let mut chunk = [0; 1024];
        let size = stream.read(&mut chunk).await.unwrap();
        assert!(size > 0, "Cloud HTTP request ended before body");
        bytes.extend_from_slice(&chunk[..size]);
    }
    let body = if content_length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
    };
    let (answer, response) = oneshot::channel();
    requested
        .send(CloudRequest {
            method,
            path,
            authorization,
            body,
            answer,
        })
        .unwrap();
    let (status, body) = response.await.unwrap();
    let body = body.to_string();
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

fn cloud_command(dir: &Path, args: &[&str]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tilrd"));
    command.args(["cloud"]).args(args);
    Server::environment(&mut command, dir, "");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command.spawn().unwrap()
}

async fn command_output(child: Child) -> std::process::Output {
    tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap()
}

async fn next_request(requests: &mut mpsc::UnboundedReceiver<CloudRequest>) -> CloudRequest {
    tokio::time::timeout(TIMEOUT, requests.recv())
        .await
        .expect("timed out waiting for a Cloud HTTP request")
        .expect("Cloud HTTP listener stopped")
}

fn enroll_answer() -> Value {
    json!({
        "device_code": "device-test", "user_code": "ABCD-EFGH",
        "verify_url": "https://cloud.tilr.test/verify", "interval": 1,
    })
}

fn approved_answer() -> Value {
    json!({ "status": "approved", "machine_id": "m1", "machine_token": MACHINE_TOKEN })
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
             echo $$ >> cloudflared.launches\n\
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
async fn login_prints_the_code_and_polls_until_confirmed() {
    let (url, _, mut requests) = cloud_with_http().await;
    let dir = Server::scratch_dir();
    let child = cloud_command(&dir, &["login", "--url", &url, "--name", "test-machine"]);
    let enroll = next_request(&mut requests).await;
    assert_eq!(
        (enroll.method.as_str(), enroll.path.as_str()),
        ("POST", "/v1/enroll")
    );
    assert_eq!(enroll.body["name"], "test-machine");
    assert!(
        enroll.body["server_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert_eq!(enroll.body["platform"], env!("TILR_PLATFORM"));
    assert!(enroll.body["version"].as_str().is_some());
    enroll.respond(200, enroll_answer());

    let pending = next_request(&mut requests).await;
    assert_eq!(
        (pending.method.as_str(), pending.path.as_str()),
        ("POST", "/v1/enroll/poll")
    );
    assert_eq!(pending.body, json!({ "device_code": "device-test" }));
    pending.respond(200, json!({ "status": "pending" }));
    let confirmed = next_request(&mut requests).await;
    assert_eq!(confirmed.body, json!({ "device_code": "device-test" }));
    confirmed.respond(200, approved_answer());

    let output = command_output(child).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("To link this machine, open"), "{stdout}");
    assert!(
        stdout.contains("https://cloud.tilr.test/verify"),
        "{stdout}"
    );
    assert!(
        stdout.contains("and confirm the code ABCD-EFGH"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Linked. A running tilrd connects on its own."),
        "{stdout}"
    );
}

#[tokio::test]
async fn login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing() {
    use std::os::unix::fs::PermissionsExt;

    let (url, _, mut requests) = cloud_with_http().await;
    let linked_dir = Server::scratch_dir();
    let child = cloud_command(&linked_dir, &["login", "--url", &url]);
    next_request(&mut requests)
        .await
        .respond(200, enroll_answer());
    next_request(&mut requests)
        .await
        .respond(200, approved_answer());
    let output = command_output(child).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let link_path = linked_dir.join("state/cloud.json");
    let link: Value = serde_json::from_slice(&std::fs::read(&link_path).unwrap()).unwrap();
    assert_eq!(
        link,
        json!({ "url": url, "machine_id": "m1", "token": MACHINE_TOKEN })
    );
    assert_eq!(
        std::fs::metadata(link_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let expired_dir = Server::scratch_dir();
    let child = cloud_command(&expired_dir, &["login", "--url", &url]);
    next_request(&mut requests)
        .await
        .respond(200, enroll_answer());
    next_request(&mut requests).await.respond(404, json!({}));
    let output = command_output(child).await;
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("the code expired, nothing was linked")
    );
    assert!(!expired_dir.join("state/cloud.json").exists());
}

#[tokio::test]
async fn logout_unlinks_the_machine_whether_or_not_cloud_hears() {
    let (url, _, mut requests) = cloud_with_http().await;
    let dir = Server::scratch_dir();
    let no_link = command_output(cloud_command(&dir, &["logout"])).await;
    assert!(no_link.status.success());
    assert!(String::from_utf8_lossy(&no_link.stdout).contains("This machine is not linked."));

    let link_path = dir.join("state/cloud.json");
    std::fs::create_dir_all(link_path.parent().unwrap()).unwrap();
    let write_link = || {
        std::fs::write(
            &link_path,
            json!({ "url": url, "machine_id": "m1", "token": MACHINE_TOKEN }).to_string(),
        )
        .unwrap()
    };
    write_link();
    let child = cloud_command(&dir, &["logout"]);
    let delete = next_request(&mut requests).await;
    assert_eq!(
        (delete.method.as_str(), delete.path.as_str()),
        ("DELETE", "/v1/machine")
    );
    assert_eq!(delete.authorization, format!("Bearer {MACHINE_TOKEN}"));
    delete.respond(401, json!({}));
    let output = command_output(child).await;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Unlinked."));
    assert!(!link_path.exists());

    let unreachable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unreachable_url = format!("http://{}", unreachable.local_addr().unwrap());
    std::fs::write(
        &link_path,
        json!({ "url": unreachable_url, "machine_id": "m1", "token": MACHINE_TOKEN }).to_string(),
    )
    .unwrap();
    let child = cloud_command(&dir, &["logout"]);
    let (connection, _) = tokio::time::timeout(TIMEOUT, unreachable.accept())
        .await
        .unwrap()
        .unwrap();
    drop(connection);
    let output = command_output(child).await;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Unlinked here, but"));
    assert!(!link_path.exists());
}

#[tokio::test]
async fn the_control_connection_is_retried_and_the_tunnel_kept_meanwhile() {
    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &[]);
    let mut first = tokio::time::timeout(TIMEOUT, connections.recv())
        .await
        .unwrap()
        .unwrap();
    first.greet().await;
    first.tunnel_is("down").await;
    first.send(tunnel("abc234.tilr.test")).await;
    first.tunnel_is("up").await;
    let pid = std::fs::read_to_string(server.dir.join("cloudflared.pid")).unwrap();
    assert!(alive(pid.trim()));

    first.ws.close(None).await.unwrap();
    let mut second = tokio::time::timeout(TIMEOUT, connections.recv())
        .await
        .expect("Cloud did not see the retry")
        .unwrap();
    second.greet().await;
    second.tunnel_is("up").await;
    assert_eq!(
        std::fs::read_to_string(server.dir.join("cloudflared.pid")).unwrap(),
        pid
    );
    assert!(alive(pid.trim()));
    assert_eq!(
        std::fs::read_to_string(server.dir.join("cloudflared.launches"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn a_cloudflared_that_exits_is_reported_and_started_again() {
    let (url, mut connections) = cloud().await;
    let server = linked_server_in(Server::scratch_dir(), &url, &[], "cloudflared", "exit 7\n");
    let mut control = tokio::time::timeout(TIMEOUT, connections.recv())
        .await
        .unwrap()
        .unwrap();
    control.greet().await;
    control.tunnel_is("down").await;
    control.send(tunnel("abc234.tilr.test")).await;
    let failed = control.tunnel_is("failed").await;
    assert!(
        failed["detail"]
            .as_str()
            .unwrap()
            .contains("cloudflared stopped"),
        "{failed}"
    );
    let retried = control.tunnel_is("failed").await;
    assert!(
        retried["detail"]
            .as_str()
            .unwrap()
            .contains("cloudflared stopped"),
        "{retried}"
    );
    let launches = std::fs::read_to_string(server.dir.join("cloudflared.launches")).unwrap();
    assert_eq!(launches.lines().count(), 2, "{launches}");
}

#[tokio::test]
async fn a_link_cloud_revokes_stops_the_tunnel_until_linked_again() {
    use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

    let (url, mut connections) = cloud().await;
    let server = linked_server(&url, &[]);
    let mut control = tokio::time::timeout(TIMEOUT, connections.recv())
        .await
        .unwrap()
        .unwrap();
    control.greet().await;
    control.tunnel_is("down").await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
    let pid = std::fs::read_to_string(server.dir.join("cloudflared.pid")).unwrap();

    control
        .ws
        .close(Some(CloseFrame {
            code: CloseCode::from(4001),
            reason: "revoked".into(),
        }))
        .await
        .unwrap();
    server
        .wait_logged("does not know this machine; link it again")
        .await;
    server.wait_logged("tunnel closed").await;
    assert!(!alive(pid.trim()));
    assert!(connections.try_recv().is_err());

    let link_path = server.dir.join("state/cloud.json");
    let replacement = server.dir.join("state/cloud-relinked.json");
    std::fs::write(
        &replacement,
        json!({
            "url": url, "machine_id": "m2", "token": "tilr_m_relinked"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::rename(replacement, link_path).unwrap();
    let mut relinked = tokio::time::timeout(TIMEOUT, connections.recv())
        .await
        .expect("Cloud did not see the new link")
        .unwrap();
    assert_eq!(relinked.authorization, "Bearer tilr_m_relinked");
    relinked.greet().await;
    relinked.tunnel_is("down").await;

    let rejecting = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rejected_url = format!("http://{}", rejecting.local_addr().unwrap());
    let rejected_server = linked_server(&rejected_url, &[]);
    let (mut stream, _) = tokio::time::timeout(TIMEOUT, rejecting.accept())
        .await
        .unwrap()
        .unwrap();
    let mut request = Vec::new();
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut chunk = [0; 1024];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "control handshake ended before its headers");
        request.extend_from_slice(&chunk[..read]);
    }
    assert!(
        std::str::from_utf8(&request)
            .unwrap()
            .starts_with("GET /v1/machine/connect ")
    );
    stream
        .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    rejected_server
        .wait_logged("does not know this machine; link it again")
        .await;
}

#[tokio::test]
async fn a_server_without_a_tcp_port_does_not_connect_to_cloud() {
    let (url, mut connections) = cloud().await;
    let dir = Server::scratch_dir();
    std::fs::create_dir_all(dir.join("state")).unwrap();
    std::fs::write(
        dir.join("state/cloud.json"),
        json!({
            "url": url, "machine_id": "m1", "token": MACHINE_TOKEN
        })
        .to_string(),
    )
    .unwrap();
    let cloudflared = dir.join("cloudflared");
    let mut command = Command::new(env!("CARGO_BIN_EXE_tilrd"));
    command.args([
        "--no-listen",
        "--cloudflared",
        cloudflared.to_str().unwrap(),
    ]);
    Server::environment(&mut command, &dir, "");
    command.stdin(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let log = tokio::task::spawn_blocking(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .find(|line| line.contains("no TCP port: no tilr Cloud tunnel"))
    });
    let observed = tokio::time::timeout(TIMEOUT, log).await.unwrap().unwrap();
    assert!(observed.is_some(), "server exited without no-TCP log");
    assert!(connections.try_recv().is_err());
    assert!(!dir.join("cloudflared.pid").exists());
    child.kill().unwrap();
    child.wait().unwrap();
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
    next.wait_logged(&format!(
        "stopped a cloudflared left by an earlier server (pid {pid})"
    ))
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
    // Starting cloudflared panics.
    server.wait_logged("failpoint spawn-unblocked").await;
    std::fs::write(&failpoints, "").unwrap();

    server.signal("-USR1");
    server.wait_logged("resumed after reload").await;
    assert!(!server.logged("reload failed"));
    // The successor starts cloudflared as usual.
    let mut control = connections.recv().await.unwrap();
    control.greet().await;
    control.send(tunnel("abc234.tilr.test")).await;
    control.tunnel_is("up").await;
}
