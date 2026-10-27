//! What the driver passes on between the server and programs on the client's
//! machine, as a real client does: agent streams to a local SSH agent, and a
//! local SOCKS port whose connections go to the server's `/socks`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::session::dial;

/// What an agent relay hands back to the session: bytes of a stream from the
/// local agent, or `None` when the agent's side ended.
pub type FromAgent = (u64, Option<Bytes>);

/// Connect stream `stream_id` to the SSH agent listening at `socket` and move
/// bytes until either side ends. What the server sends comes in on `to_agent`
/// (closed: the server ended the stream).
pub async fn agent(
    stream_id: u64,
    socket: PathBuf,
    mut to_agent: mpsc::UnboundedReceiver<Bytes>,
    back: mpsc::UnboundedSender<FromAgent>,
) {
    let Ok(stream) = UnixStream::connect(&socket).await else {
        let _ = back.send((stream_id, None));
        return;
    };
    let (mut read, mut write) = stream.into_split();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            data = to_agent.recv() => match data {
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
                    if back.send((stream_id, Some(Bytes::copy_from_slice(&buf[..n])))).is_err() {
                        return;
                    }
                }
            },
        }
    }
    let _ = back.send((stream_id, None));
}

/// Counters of a local SOCKS port.
#[derive(Default)]
pub struct SocksStats {
    pub sessions: AtomicU64,
    pub open: AtomicU64,
    pub failed: AtomicU64,
    pub bytes_up: AtomicU64,
    pub bytes_down: AtomicU64,
}

impl SocksStats {
    pub fn to_json(&self) -> Value {
        let get = |n: &AtomicU64| n.load(Ordering::Relaxed);
        json!({
            "sessions": get(&self.sessions), "open": get(&self.open), "failed": get(&self.failed),
            "bytes_up": get(&self.bytes_up), "bytes_down": get(&self.bytes_down),
        })
    }
}

/// Listen on `addr` (127.0.0.1:0 for any port) and carry each connection, a
/// SOCKS5 client such as curl, to `url` (the server's `/socks`) as it is:
/// the WebSocket carries the SOCKS bytes. Returns the port.
pub async fn socks_listen(addr: &str, url: String, stats: Arc<SocksStats>) -> Result<u16> {
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("listen on {addr}"))?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let _ = tcp.set_nodelay(true);
            tokio::spawn(socks_bridge(tcp, url.clone(), Arc::clone(&stats)));
        }
    });
    Ok(port)
}

async fn socks_bridge(tcp: TcpStream, url: String, stats: Arc<SocksStats>) {
    stats.sessions.fetch_add(1, Ordering::Relaxed);
    let Ok(ws) = dial(&url, None).await else {
        stats.failed.fetch_add(1, Ordering::Relaxed);
        return;
    };
    stats.open.fetch_add(1, Ordering::Relaxed);
    let (mut ws_tx, mut ws_rx) = ws.split();
    let (mut read, mut write) = tcp.into_split();
    let up = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let piece = Bytes::copy_from_slice(&buf[..n]);
                    if ws_tx.send(Message::Binary(piece)).await.is_err() {
                        break;
                    }
                    stats.bytes_up.fetch_add(n as u64, Ordering::Relaxed);
                }
            }
        }
        let _ = ws_tx.close().await;
    };
    let down = async {
        while let Some(Ok(message)) = ws_rx.next().await {
            match message {
                Message::Binary(bytes) => {
                    if write.write_all(&bytes).await.is_err() {
                        break;
                    }
                    stats
                        .bytes_down
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = write.shutdown().await;
    };
    // The server's side ending ends the session; the client's only half of
    // it, as a SOCKS proxy's connection does.
    tokio::pin!(up, down);
    tokio::select! {
        () = &mut up => down.await,
        () = &mut down => {}
    }
    stats.open.fetch_sub(1, Ordering::Relaxed);
}
