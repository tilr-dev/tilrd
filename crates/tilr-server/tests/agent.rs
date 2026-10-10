//! Use cases: SSH agent forwarding. A program in a pane connects to the
//! socket in its `SSH_AUTH_SOCK` and talks to the agent on the client's
//! side, through `StreamOpen` / `StreamData` / `StreamClose`.

mod common;

use common::*;
use tilr_proto::{StreamClose, StreamData, StreamOpen};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

fn start_with_config(config: &str, args: &[&str]) -> Server {
    let dir = Server::scratch_dir();
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/server.toml"), config).unwrap();
    Server::start_in(dir, Path::new(env!("CARGO_BIN_EXE_tilr-server")), args)
}

/// Connect and say hello as a client that serves the agent.
async fn agent_client(server: &Server) -> Client {
    let mut client = Client::connect_silent(server).await;
    client
        .hello_with(PROTOCOL_VERSION, &["clipboard", "host-events", "agent"])
        .await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::HelloAck(_) => Some(()),
            _ => None,
        })
        .await;
    client
}

/// The pane's socket. Where the scratch directory's path is too long for a
/// socket (macOS) it is under /tmp, which a killed server leaves behind:
/// removed when this goes.
struct Socket(PathBuf);

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `$SSH_AUTH_SOCK` as the shell in the pane has it; empty if unset.
async fn auth_sock(server: &Server, client: &mut Client, pane_id: PaneId) -> String {
    let file = server.dir.join(format!("auth-sock-{pane_id}"));
    let command = format!("printenv SSH_AUTH_SOCK > '{}'", file.display());
    client.run(pane_id, &command).await;
    std::fs::read_to_string(&file).unwrap().trim().to_owned()
}

/// The pane's socket, checked to be where the server keeps them.
async fn pane_socket(server: &Server, client: &mut Client, pane_id: PaneId) -> Socket {
    let path = PathBuf::from(auth_sock(server, client, pane_id).await);
    assert!(
        path.ends_with(format!("agent/{pane_id}.sock")),
        "{}",
        path.display()
    );
    Socket(path)
}

/// Subscribe to the pane and wait until the server has taken it in: the
/// first frame has come.
async fn watch(client: &mut Client, pane_id: PaneId) {
    client.subscribe(pane_id).await;
    let pane = pane_id.to_string();
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Frame(frame) if frame.pane_id == pane => Some(()),
            _ => None,
        })
        .await;
}

/// Unsubscribe and wait until the server has taken it in.
async fn unwatch(client: &mut Client, pane_id: PaneId, nonce: u64) {
    client
        .send(Msg::Unsubscribe(Unsubscribe {
            pane_id: pane_id.to_string(),
        }))
        .await;
    assert!(no_stream_message(client, nonce).await);
}

async fn stream_open(client: &mut Client) -> StreamOpen {
    client
        .wait_for(|msg| match msg {
            server_message::Msg::StreamOpen(open) => Some(open.clone()),
            server_message::Msg::StreamData(_) | server_message::Msg::StreamClose(_) => {
                panic!("stream message before StreamOpen: {msg:?}")
            }
            _ => None,
        })
        .await
}

/// `len` bytes of the stream, in as many messages as they come in.
async fn stream_bytes(client: &mut Client, stream_id: u64, len: usize) -> Vec<u8> {
    let mut got = Vec::new();
    while got.len() < len {
        let data = client
            .wait_for(|msg| match msg {
                server_message::Msg::StreamData(data) if data.stream_id == stream_id => {
                    Some(data.data.clone())
                }
                server_message::Msg::StreamClose(close) if close.stream_id == stream_id => {
                    panic!("stream closed after {} bytes", got.len())
                }
                _ => None,
            })
            .await;
        got.extend_from_slice(&data);
    }
    got
}

async fn stream_closed(client: &mut Client, stream_id: u64) {
    client
        .wait_for(|msg| match msg {
            server_message::Msg::StreamClose(close) if close.stream_id == stream_id => Some(()),
            _ => None,
        })
        .await;
}

async fn send_data(client: &mut Client, stream_id: u64, data: &[u8]) {
    client
        .send(Msg::StreamData(StreamData {
            stream_id,
            data: Bytes::copy_from_slice(data),
        }))
        .await;
}

/// Read the socket until it ends; panics if it does not end soon.
async fn read_to_end(socket: &mut UnixStream) -> Vec<u8> {
    let mut got = Vec::new();
    let read = tokio::time::timeout(TIMEOUT, socket.read_to_end(&mut got)).await;
    match read.expect("the socket did not end") {
        Ok(_) => {}
        // A connection closed with nothing to read may also be reset.
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => {}
        Err(err) => panic!("read: {err}"),
    }
    got
}

/// Whether no stream message arrives before the answer to a ping.
async fn no_stream_message(client: &mut Client, nonce: u64) -> bool {
    client.ping(nonce).await;
    client
        .wait_for(|msg| match msg {
            server_message::Msg::Pong(pong) if pong.nonce == nonce => Some(true),
            server_message::Msg::StreamOpen(_)
            | server_message::Msg::StreamData(_)
            | server_message::Msg::StreamClose(_) => Some(false),
            _ => None,
        })
        .await
}

#[tokio::test]
async fn the_agent_is_forwarded_unless_turned_off() {
    let server = Server::start(&[]);
    let mut client = Client::connect_silent(&server).await;
    let hello = client.hello().await;
    assert!(
        hello.features.contains(&"agent".to_owned()),
        "{:?}",
        hello.features
    );

    let flag = Server::start(&["--no-agent"]);
    let config = start_with_config("agent = false\n", &[]);
    for server in [&flag, &config] {
        let mut client = Client::connect_silent(server).await;
        let hello = client.hello().await;
        assert!(
            !hello.features.contains(&"agent".to_owned()),
            "{:?}",
            hello.features
        );
        // No socket, nothing in the pane's environment.
        let pane_id = client.open_pane().await;
        assert_eq!(auth_sock(server, &mut client, pane_id).await, "");
        assert!(!server.dir.join("runtime/agent").exists());
    }
}

#[tokio::test]
async fn a_pane_has_ssh_auth_sock_pointing_at_its_own_socket() {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    let server = Server::start(&[]);
    let mut client = agent_client(&server).await;
    let first = client.open_pane().await;
    let socket = pane_socket(&server, &mut client, first).await;
    let meta = std::fs::symlink_metadata(&socket.0).unwrap();
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let dir = std::fs::metadata(socket.0.parent().unwrap()).unwrap();
    assert_eq!(dir.permissions().mode() & 0o777, 0o700);

    // Each pane its own.
    let second = client.open_pane().await;
    let other = pane_socket(&server, &mut client, second).await;
    assert_ne!(socket.0, other.0);

    // It goes with the pane.
    client.type_line(first, "exit").await;
    client.pane_exit_code(first).await;
    eventually("the socket is removed", || !socket.0.exists()).await;
    assert!(other.0.exists());
}

#[tokio::test]
async fn a_program_talks_to_the_agent_of_the_client_through_the_panes_socket() {
    let server = Server::start(&[]);
    let mut client = agent_client(&server).await;
    let pane_id = client.open_pane().await;
    let socket = pane_socket(&server, &mut client, pane_id).await;

    // The program asks, the client answers; then the program hangs up.
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    let open = stream_open(&mut client).await;
    assert_eq!(open.kind, "ssh-agent");
    assert_eq!(open.pane_id, pane_id.to_string());
    program.write_all(b"\0\0\0\x01\x0b").await.unwrap();
    assert_eq!(
        stream_bytes(&mut client, open.stream_id, 5).await,
        b"\0\0\0\x01\x0b"
    );
    send_data(&mut client, open.stream_id, b"\0\0\0\x05\x0c\0\0\0\0").await;
    let mut answer = [0u8; 9];
    program.read_exact(&mut answer).await.unwrap();
    assert_eq!(&answer, b"\0\0\0\x05\x0c\0\0\0\0");

    // More than one message's worth goes through whole, both ways.
    let big = image(200 * 1024);
    program.write_all(&big).await.unwrap();
    assert_eq!(
        stream_bytes(&mut client, open.stream_id, big.len()).await,
        big
    );
    for chunk in big.chunks(64 * 1024) {
        send_data(&mut client, open.stream_id, chunk).await;
    }
    let mut back = vec![0u8; big.len()];
    program.read_exact(&mut back).await.unwrap();
    assert_eq!(back, big);

    drop(program);
    stream_closed(&mut client, open.stream_id).await;

    // A second connection is a new stream; the client closes it, the
    // program reads what was sent before the close, then the end.
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    let second = stream_open(&mut client).await;
    assert_ne!(second.stream_id, open.stream_id);
    send_data(&mut client, second.stream_id, b"last words").await;
    client
        .send(Msg::StreamClose(StreamClose {
            stream_id: second.stream_id,
        }))
        .await;
    assert_eq!(read_to_end(&mut program).await, b"last words");

    // Data for a stream that is closed, or was never opened, is ignored.
    send_data(&mut client, second.stream_id, b"late").await;
    send_data(&mut client, 999, b"nobody").await;
    assert!(no_stream_message(&mut client, 1).await);
}

#[tokio::test]
async fn with_no_agent_client_attached_the_program_finds_no_agent() {
    let server = Server::start(&[]);
    // Watched by a client that cannot serve the agent.
    let mut plain = Client::connect(&server).await;
    let pane_id = plain.open_pane().await;
    let socket = pane_socket(&server, &mut plain, pane_id).await;
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    assert_eq!(read_to_end(&mut program).await, b"");
    assert!(no_stream_message(&mut plain, 1).await);

    // A client that can, but does not watch the pane.
    let mut agent = agent_client(&server).await;
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    assert_eq!(read_to_end(&mut program).await, b"");
    assert!(no_stream_message(&mut agent, 2).await);

    // It watched the pane and stopped.
    watch(&mut agent, pane_id).await;
    unwatch(&mut agent, pane_id, 3).await;
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    assert_eq!(read_to_end(&mut program).await, b"");
    assert!(no_stream_message(&mut agent, 4).await);
}

#[tokio::test]
async fn the_program_reads_the_end_when_the_agent_client_disconnects() {
    let server = Server::start(&[]);
    let mut client = agent_client(&server).await;
    let pane_id = client.open_pane().await;
    let socket = pane_socket(&server, &mut client, pane_id).await;
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    let open = stream_open(&mut client).await;
    send_data(&mut client, open.stream_id, b"hello").await;
    let mut got = [0u8; 5];
    program.read_exact(&mut got).await.unwrap();

    drop(client);
    assert_eq!(read_to_end(&mut program).await, b"");

    // The socket stays for the next client.
    let mut client = agent_client(&server).await;
    watch(&mut client, pane_id).await;
    let _program = UnixStream::connect(&socket.0).await.unwrap();
    stream_open(&mut client).await;
}

#[tokio::test]
async fn the_client_that_subscribed_last_serves_the_agent() {
    let server = Server::start(&[]);
    let mut first = agent_client(&server).await;
    let pane_id = first.open_pane().await;
    let socket = pane_socket(&server, &mut first, pane_id).await;
    let mut second = agent_client(&server).await;
    watch(&mut second, pane_id).await;
    // A later subscriber that cannot serve the agent does not count.
    let mut plain = Client::connect(&server).await;
    watch(&mut plain, pane_id).await;

    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    let open = stream_open(&mut second).await;
    assert_eq!(open.pane_id, pane_id.to_string());
    program.write_all(b"ping").await.unwrap();
    assert_eq!(stream_bytes(&mut second, open.stream_id, 4).await, b"ping");
    assert!(no_stream_message(&mut first, 1).await);
    assert!(no_stream_message(&mut plain, 2).await);

    // Once it leaves the pane, the first one does.
    unwatch(&mut second, pane_id, 3).await;
    let _program = UnixStream::connect(&socket.0).await.unwrap();
    let again = stream_open(&mut first).await;
    assert_ne!(again.stream_id, open.stream_id);
}

#[tokio::test]
async fn the_panes_socket_works_after_a_reload() {
    let server = Server::start(&[]);
    let mut client = agent_client(&server).await;
    let pane_id = client.open_pane().await;
    let socket = pane_socket(&server, &mut client, pane_id).await;
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    stream_open(&mut client).await;

    // The connection open across the reload ends with it.
    let plain = reload(&server, client).await;
    drop(plain);
    assert_eq!(read_to_end(&mut program).await, b"");
    assert!(server.logged("resumed after reload with 1 pane(s)"));

    let mut client = agent_client(&server).await;
    watch(&mut client, pane_id).await;
    // Same shell, same path, a socket again.
    assert_eq!(
        PathBuf::from(auth_sock(&server, &mut client, pane_id).await),
        socket.0
    );
    let mut program = UnixStream::connect(&socket.0).await.unwrap();
    let open = stream_open(&mut client).await;
    assert_eq!(open.pane_id, pane_id.to_string());
    program.write_all(b"after").await.unwrap();
    assert_eq!(stream_bytes(&mut client, open.stream_id, 5).await, b"after");
    send_data(&mut client, open.stream_id, b"ok").await;
    let mut got = [0u8; 2];
    program.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"ok");
}
