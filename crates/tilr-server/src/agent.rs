//! SSH agent forwarding, as `ssh -A` does it: a program in a pane talks to
//! the SSH agent of the user's computer, on the client's side.
//!
//! Every pane gets a Unix socket of its own, `agent/<pane id>.sock` in the
//! runtime directory (see [`socket_path`]), and its path as `SSH_AUTH_SOCK`.
//! The socket is made with the pane and again for every pane after a reload,
//! at the same path, and goes when the pane does.
//!
//! A program that connects to it (`ssh`, `ssh-add`, `git`) gets a stream to
//! one client: of the clients subscribed to the pane that announced "agent"
//! in their `Hello`, the one that subscribed last. The client is sent
//! `StreamOpen`, then the bytes go both ways as `StreamData` until either
//! side closes (`StreamClose`). The server does not read them. With no such
//! client attached the connection is closed at once, which `ssh` takes for
//! an agent without keys.
//!
//! The supervisor's actor owns an [`Agent`]: the sockets and the table of
//! open streams. Accepting and moving bytes happen in tasks, which tell the
//! actor what happened through the [`Sink`] as an [`Output`].

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tilr_proto::{ServerMessage, StreamClose, StreamData, StreamOpen, server_message};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::outbox::ToClient;
use crate::{clipboard, host};

/// Set in every pane to the path of its socket.
pub const SOCK_ENV: &str = "SSH_AUTH_SOCK";
/// `StreamOpen.kind` of an agent stream.
pub const KIND: &str = "ssh-agent";
/// Most bytes in one `StreamData` the server sends.
const CHUNK: usize = 64 * 1024;
/// Streams one client may have open at once; a program connecting beyond
/// that is closed at once.
const MAX_STREAMS_PER_CLIENT: usize = 64;
/// `StreamData` messages from the client waiting to be written to one
/// program, at most 64 KiB each as a client should send them: about 4 MiB.
/// A client that runs that far ahead of the program loses the stream
/// (`StreamClose`). The other way, the socket is read only as fast as the
/// client's bulk lane has room (`outbox::BULK_BYTES` for all its streams
/// and file reads), so a slow client holds the program up rather than
/// memory growing.
const SOCKET_QUEUE: usize = 64;

/// What the tasks hand back to the supervisor's actor.
pub enum Output {
    /// A program connected to a pane's socket.
    Connected { pane_id: Uuid, stream: UnixStream },
    /// The program closed its connection; the client has been sent
    /// `StreamClose`.
    Ended { stream_id: u64 },
}

/// Where the tasks' outputs go: to the supervisor's actor.
pub type Sink = Arc<dyn Fn(Output) + Send + Sync>;

pub struct Agent {
    sink: Sink,
    sockets: HashMap<Uuid, Socket>,
    streams: HashMap<u64, Stream>,
    /// The id the next stream gets; never reused while the server runs.
    next_stream: u64,
}

/// The listening socket of one pane.
struct Socket {
    path: PathBuf,
    accept: JoinHandle<()>,
    /// The clients subscribed to the pane, in the order they subscribed:
    /// the last one is asked first.
    subscribers: Vec<u64>,
}

/// An open stream, one program's connection.
struct Stream {
    client: u64,
    /// Bytes from the client for the program. Dropping it closes the
    /// connection once what is queued has been written.
    to_socket: mpsc::Sender<Bytes>,
    task: JoinHandle<()>,
}

/// Where the socket of a pane is: `agent/<pane id>.sock` in the runtime
/// directory, or under `/tmp/tilr-<uid>` where that path is too long for a
/// socket (see [`host::runtime_socket`]).
pub fn socket_path(pane_id: Uuid) -> PathBuf {
    host::runtime_socket(&Path::new("agent").join(format!("{pane_id}.sock")))
}

impl Agent {
    pub fn new(sink: Sink) -> Self {
        Self {
            sink,
            sockets: HashMap::new(),
            streams: HashMap::new(),
            next_stream: 1,
        }
    }

    /// Make the socket of a pane and return its path, for the pane's
    /// `SSH_AUTH_SOCK`. A socket a server left behind at that path is
    /// replaced. `None`, with a warning, if it cannot be made: the pane
    /// goes without the agent.
    pub fn listen(&mut self, pane_id: Uuid) -> Option<PathBuf> {
        let path = socket_path(pane_id);
        match bind(&path) {
            Ok(listener) => {
                let accept = tokio::spawn(accept(pane_id, listener, Arc::clone(&self.sink)));
                tracing::debug!(pane = %pane_id, path = %path.display(), "agent socket");
                self.sockets.insert(
                    pane_id,
                    Socket {
                        path: path.clone(),
                        accept,
                        subscribers: Vec::new(),
                    },
                );
                Some(path)
            }
            Err(err) => {
                tracing::warn!(pane = %pane_id, "no agent in the pane: {err:#}");
                None
            }
        }
    }

    /// The pane is gone: so is its socket. Streams already open go on.
    pub fn close_pane(&mut self, pane_id: Uuid) {
        if let Some(socket) = self.sockets.remove(&pane_id) {
            socket.accept.abort();
            let _ = std::fs::remove_file(&socket.path);
        }
    }

    /// Remove every socket, when the server stops for good.
    pub fn remove_all(&mut self) {
        let panes: Vec<Uuid> = self.sockets.keys().copied().collect();
        for pane_id in panes {
            self.close_pane(pane_id);
        }
    }

    pub fn subscribed(&mut self, pane_id: Uuid, client: u64) {
        if let Some(socket) = self.sockets.get_mut(&pane_id) {
            socket.subscribers.retain(|&id| id != client);
            socket.subscribers.push(client);
        }
    }

    pub fn unsubscribed(&mut self, pane_id: Uuid, client: u64) {
        if let Some(socket) = self.sockets.get_mut(&pane_id) {
            socket.subscribers.retain(|&id| id != client);
        }
    }

    /// A program connected to the socket of `pane_id`: open a stream to the
    /// client that subscribed to the pane last of those `agent_client`
    /// gives a sender for (the ones with the "agent" feature), or close the
    /// connection if there is none.
    pub fn connected(
        &mut self,
        pane_id: Uuid,
        stream: UnixStream,
        agent_client: impl Fn(u64) -> Option<ToClient>,
    ) {
        let Some(socket) = self.sockets.get(&pane_id) else {
            return;
        };
        let Some((client, to_client)) = socket
            .subscribers
            .iter()
            .rev()
            .find_map(|&client| Some((client, agent_client(client)?)))
        else {
            tracing::debug!(pane = %pane_id, "agent asked for with no client attached");
            return;
        };
        let open = self.streams.values().filter(|s| s.client == client).count();
        if open >= MAX_STREAMS_PER_CLIENT {
            tracing::warn!(
                client,
                pane = %pane_id,
                "agent connection refused: {MAX_STREAMS_PER_CLIENT} streams open already"
            );
            return;
        }
        let stream_id = self.next_stream;
        self.next_stream += 1;
        let opened = to_client.try_send(
            encode(server_message::Msg::StreamOpen(StreamOpen {
                stream_id,
                kind: KIND.into(),
                pane_id: pane_id.to_string(),
            }))
            .into(),
        );
        if opened.is_err() {
            tracing::debug!(
                client,
                "agent connection dropped: the client is not taking packets"
            );
            return;
        }
        tracing::debug!(client, pane = %pane_id, stream = stream_id, "agent stream open");
        let (to_socket, from_client) = mpsc::channel(SOCKET_QUEUE);
        let task = tokio::spawn(pump(
            stream_id,
            stream,
            from_client,
            to_client,
            Arc::clone(&self.sink),
        ));
        self.streams.insert(
            stream_id,
            Stream {
                client,
                to_socket,
                task,
            },
        );
    }

    /// Bytes from a client for the program at the other end of a stream.
    /// Returns a message for the client if the stream had to be closed.
    pub fn on_data(&mut self, client: u64, data: StreamData) -> Option<server_message::Msg> {
        let Some(stream) = self.owned(client, data.stream_id) else {
            tracing::debug!(
                client,
                stream = data.stream_id,
                "data for a stream not the client's"
            );
            return None;
        };
        match stream.to_socket.try_send(data.data) {
            Ok(()) => None,
            // The task is ending; it says so itself.
            Err(mpsc::error::TrySendError::Closed(_)) => None,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    client,
                    stream = data.stream_id,
                    "agent stream closed: the client sends faster than the program reads"
                );
                if let Some(stream) = self.streams.remove(&data.stream_id) {
                    stream.task.abort();
                }
                Some(server_message::Msg::StreamClose(StreamClose {
                    stream_id: data.stream_id,
                }))
            }
        }
    }

    /// The client is done with a stream: the program reads what is still
    /// queued, then end of file.
    pub fn on_close(&mut self, client: u64, close: StreamClose) {
        if self.owned(client, close.stream_id).is_none() {
            tracing::debug!(
                client,
                stream = close.stream_id,
                "close of a stream not the client's"
            );
            return;
        }
        // Dropping the sender lets the task write what is queued and stop.
        self.streams.remove(&close.stream_id);
        tracing::debug!(
            client,
            stream = close.stream_id,
            "agent stream closed by the client"
        );
    }

    /// The program closed a stream; the task has told the client.
    pub fn ended(&mut self, stream_id: u64) {
        if self.streams.remove(&stream_id).is_some() {
            tracing::debug!(stream = stream_id, "agent stream closed by the program");
        }
    }

    /// The client is gone: so are its streams, the programs read end of
    /// file.
    pub fn drop_client(&mut self, client: u64) {
        for socket in self.sockets.values_mut() {
            socket.subscribers.retain(|&id| id != client);
        }
        self.streams.retain(|&stream_id, stream| {
            if stream.client != client {
                return true;
            }
            stream.task.abort();
            tracing::debug!(
                client,
                stream = stream_id,
                "agent stream closed: client gone"
            );
            false
        });
    }

    fn owned(&self, client: u64, stream_id: u64) -> Option<&Stream> {
        self.streams
            .get(&stream_id)
            .filter(|stream| stream.client == client)
    }
}

/// Listen at `path` in a directory only the user can enter, the socket
/// itself readable and writable by the user only.
fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    // The runtime directory above `agent/`, which must be the user's own
    // too.
    if let Some(runtime_dir) = path.parent().and_then(Path::parent) {
        clipboard::private_dir(runtime_dir)?;
    }
    let listener = host::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

async fn accept(pane_id: Uuid, listener: UnixListener, sink: Sink) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => sink(Output::Connected { pane_id, stream }),
            Err(err) => {
                tracing::warn!(pane = %pane_id, "agent socket: accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Move bytes between a program and its client until one side is done.
/// When the program closes, the client is sent `StreamClose`; when the
/// client does (the sender of `from_client` dropped), the program reads end
/// of file after what was queued.
async fn pump(
    stream_id: u64,
    stream: UnixStream,
    mut from_client: mpsc::Receiver<Bytes>,
    to_client: ToClient,
    sink: Sink,
) {
    let (mut read, mut write) = stream.into_split();
    let mut buf = vec![0u8; CHUNK];
    loop {
        tokio::select! {
            data = from_client.recv() => match data {
                Some(data) => {
                    if write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                None => return,
            },
            read = read.read(&mut buf) => match read {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let packet = encode(server_message::Msg::StreamData(StreamData {
                        stream_id,
                        data: Bytes::copy_from_slice(&buf[..n]),
                    }));
                    if to_client.send_bulk(packet).await.is_err() {
                        return;
                    }
                }
            },
        }
    }
    // By the bulk lane too: after the stream's last bytes.
    let close = encode(server_message::Msg::StreamClose(StreamClose { stream_id }));
    let _ = to_client.send_bulk(close).await;
    sink(Output::Ended { stream_id });
}

fn encode(msg: server_message::Msg) -> Bytes {
    tilr_proto::encode(&ServerMessage::from(msg))
}
