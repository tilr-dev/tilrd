//! SOCKS sessions over the server's `/socks` WebSocket: a connection the
//! server makes, on the client's behalf, to something listening on its
//! machine. Each is a WebSocket of its own beside the main connection,
//! read only while a command waits on it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use crate::session::{Ws, dial, ms};

/// Largest binary message sent.
const PIECE: usize = 16 * 1024;

pub struct Socks {
    ws: Ws,
    unread: VecDeque<u8>,
    /// Why the server's side ended, once it has.
    pub ended: Option<String>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl Socks {
    /// Open `url` (ws://host:port/socks?token=...) and ask for a connection
    /// to `host`:`port` on the server's machine (SOCKS5, no authentication,
    /// the name resolved there).
    pub async fn open(url: &str, host: &str, port: u16, timeout: Duration) -> Result<Self> {
        let work = async {
            let mut socks = Socks {
                ws: dial(url, None).await?,
                unread: VecDeque::new(),
                ended: None,
                bytes_in: 0,
                bytes_out: 0,
            };
            socks.send(&[5, 1, 0]).await?;
            let choice = socks.read_exact(2).await?;
            if choice != [5, 0] {
                bail!("server picked SOCKS method {choice:?}");
            }
            let mut request = vec![5, 1, 0, 3, host.len() as u8];
            request.extend(host.as_bytes());
            request.extend(port.to_be_bytes());
            socks.send(&request).await?;
            let head = socks.read_exact(4).await?;
            if head[1] != 0 {
                bail!("SOCKS connect to {host}:{port} refused: reply {}", head[1]);
            }
            let rest = match head[3] {
                1 => 4 + 2,
                4 => 16 + 2,
                3 => socks.read_exact(1).await?[0] as usize + 2,
                other => bail!("SOCKS reply with address type {other}"),
            };
            socks.read_exact(rest).await?;
            socks.bytes_out = 0;
            socks.bytes_in = 0;
            Ok(socks)
        };
        tokio::time::timeout(timeout, work)
            .await
            .map_err(|_| anyhow::anyhow!("SOCKS handshake took over {} ms", timeout.as_millis()))?
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<()> {
        for piece in data.chunks(PIECE) {
            self.ws
                .send(Message::Binary(Bytes::copy_from_slice(piece)))
                .await?;
            self.bytes_out += piece.len() as u64;
        }
        Ok(())
    }

    async fn read_exact(&mut self, n: usize) -> Result<Vec<u8>> {
        while self.unread.len() < n {
            if !self.read_more().await {
                bail!(
                    "SOCKS connection ended: {}",
                    self.ended.clone().unwrap_or_default()
                );
            }
        }
        Ok(self.unread.drain(..n).collect())
    }

    /// One more message into `unread`; false once the server's side ended.
    async fn read_more(&mut self) -> bool {
        if self.ended.is_some() {
            return false;
        }
        match self.ws.next().await {
            Some(Ok(Message::Binary(bytes))) => {
                self.bytes_in += bytes.len() as u64;
                self.unread.extend(bytes);
            }
            Some(Ok(Message::Close(frame))) => {
                self.ended = Some(format!(
                    "closed {}",
                    frame
                        .map(|f| format!("{} {}", f.code, f.reason))
                        .unwrap_or_default()
                ));
            }
            Some(Ok(_)) => {}
            Some(Err(err)) => self.ended = Some(format!("read failed: {err}")),
            None => self.ended = Some("connection ended".into()),
        }
        true
    }

    /// Read until `want` bytes are here, the end, or the time is up; takes
    /// what came.
    pub async fn recv(&mut self, want: usize, timeout: Duration) -> Value {
        let started = Instant::now();
        let deadline = started + timeout;
        while self.unread.len() < want && self.ended.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || tokio::time::timeout(left, self.read_more()).await.is_err() {
                break;
            }
        }
        let got: Vec<u8> = self.unread.drain(..).collect();
        let text = String::from_utf8_lossy(&got[..got.len().min(4096)]).into_owned();
        json!({
            "received": got.len(), "text": text, "ended": self.ended,
            "bytes_in": self.bytes_in, "ms": ms(started.elapsed()),
        })
    }

    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.ws.close(None)).await;
    }
}
