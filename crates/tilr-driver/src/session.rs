//! One client connection to a tilr server and what it knows: the screens of
//! the panes it watches, keepalive and reconnecting, the streams the server
//! opened to it, and how long its input and frames took.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tilr_proto::{
    ClientMessage, ClipboardWrite, Hello, PROTOCOL_VERSION, ServerMessage, StreamClose, StreamData,
    Subscribe, client_message::Msg, server_message,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::sleep_until;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::exec;
use crate::relay::{self, FromAgent};
use crate::screen::Screen;
use crate::socks::Socks;
use crate::trace::Trace;
use crate::transfer::{Download, Progress, Upload, hex};
use crate::{emit, now_ms};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_PAUSE: Duration = Duration::from_millis(200);

/// Where to connect and how to introduce ourselves.
#[derive(Clone)]
pub struct Target {
    pub url: String,
    pub token: String,
    pub origin: Option<String>,
    pub features: Vec<String>,
    pub timeout: Duration,
    /// `Hello.client_id`, the same on every connection.
    pub client_id: String,
}

pub struct Keepalive {
    /// Ping this often; also how often silence is checked.
    pub interval: Duration,
    /// Silence (nothing at all from the server) after which the connection
    /// is taken for dead.
    pub timeout: Duration,
    pub auto_reconnect: bool,
    /// How long to keep trying to reconnect after a loss.
    pub reconnect_timeout: Duration,
    /// How long one reconnect attempt may take (a slow, lossy network needs
    /// more than a few round trips' worth).
    pub attempt_timeout: Duration,
    /// Only answers to our pings (Pong) count as signs of life, not any data:
    /// a connection that still brings frames but no longer takes what we
    /// send is dead too.
    pub pong_only: bool,
    /// A connection taken for dead is left open and unread, as by an app
    /// that was suspended, instead of closed.
    pub leak_dead: bool,
}

/// A stream the server opened to us (StreamOpen): a program's connection
/// to the agent.
pub struct Stream {
    pub kind: String,
    pub pane: String,
    pub opened_ms: f64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Of the bytes that came in, in order.
    pub sha_in: Sha256,
    /// To the relay of a local agent (`AgentMode::Socket`).
    to_agent: Option<mpsc::UnboundedSender<Bytes>>,
}

impl Stream {
    pub fn to_json(&self, id: u64) -> Value {
        json!({
            "stream": id, "kind": self.kind, "pane": self.pane, "opened_ms": self.opened_ms,
            "bytes_in": self.bytes_in, "bytes_out": self.bytes_out,
            "sha256_in": hex(&self.sha_in.clone().finalize()),
        })
    }
}

/// What woke [`Session::read`].
enum Next {
    Message(Option<Result<Message, tokio_tungstenite::tungstenite::Error>>),
    Agent(u64, Option<Bytes>),
    Wake,
}

/// What the driver does with the streams the server opens to it.
#[derive(Clone, Debug)]
pub enum AgentMode {
    /// Echo every byte back: a toy agent.
    Echo,
    /// Take the bytes and answer nothing (counted and hashed).
    Sink,
    /// Pass them on to the SSH agent listening at this path, as a client
    /// forwarding the user's agent does.
    Socket(PathBuf),
}

pub struct Session {
    ws: Option<Ws>,
    pub target: Option<Target>,
    pub screens: HashMap<String, Screen>,
    pub subscribed: BTreeSet<String>,
    seq_base: u64,
    seq_n: u64,
    last_rx: Instant,
    last_pong: Instant,
    keepalive: Option<Keepalive>,
    next_ping: Instant,
    nonce: u64,
    /// The connection was lost and is to be made again (auto reconnect).
    lost_at: Option<(Instant, String)>,
    attempts: u32,
    next_request: u64,
    /// Counts the connections made: a transfer that sees it change lost its
    /// connection on the way.
    pub generation: u64,
    pub trace: Trace,
    pub streams: BTreeMap<u64, Stream>,
    pub agent: AgentMode,
    /// Messages to send before the next read (echoes, a local agent's bytes).
    outbox: VecDeque<Msg>,
    relay_tx: mpsc::UnboundedSender<FromAgent>,
    relay_rx: mpsc::UnboundedReceiver<FromAgent>,
    /// HelloAck.features of the connection.
    pub features: Vec<String>,
    pub uploads: Vec<Upload>,
    pub downloads: Vec<Download>,
    /// Outcomes of transfers, by transfer id, until taken.
    pub finished: HashMap<u64, Value>,
    pub next_transfer: u64,
    /// Programs run by Exec, by exec id.
    pub execs: BTreeMap<u64, exec::Run>,
    /// Answers to ExecStart by request id: the exec id, or why it did not start.
    pub exec_started: HashMap<u64, Result<u64>>,
    /// Of the local SOCKS ports (`socks_listen`).
    pub socks_stats: Arc<relay::SocksStats>,
    pub socks: BTreeMap<u64, Socks>,
    pub next_socks: u64,
    /// Connections given up on but left open (Keepalive.leak_dead).
    leaked: Vec<Ws>,
    /// The copy made in a pane that is coming in (`ClipboardWrite`).
    copy: Option<IncomingCopy>,
    /// The newest `ClipboardWrite.transfer_id` seen on this connection.
    newest_copy: u64,
}

/// A copy made in a pane, as its chunks come.
struct IncomingCopy {
    first: ClipboardWrite,
    data: Vec<u8>,
    chunks: usize,
}

impl Session {
    pub fn new() -> Self {
        let (relay_tx, relay_rx) = mpsc::unbounded_channel();
        Self {
            ws: None,
            target: None,
            screens: HashMap::new(),
            subscribed: BTreeSet::new(),
            seq_base: 0,
            seq_n: 0,
            last_rx: Instant::now(),
            last_pong: Instant::now(),
            keepalive: None,
            next_ping: Instant::now(),
            nonce: 0,
            lost_at: None,
            attempts: 0,
            next_request: 0,
            generation: 0,
            trace: Trace::default(),
            streams: BTreeMap::new(),
            agent: AgentMode::Echo,
            outbox: VecDeque::new(),
            relay_tx,
            relay_rx,
            features: Vec::new(),
            uploads: Vec::new(),
            downloads: Vec::new(),
            finished: HashMap::new(),
            next_transfer: 0,
            execs: BTreeMap::new(),
            exec_started: HashMap::new(),
            socks_stats: Arc::default(),
            socks: BTreeMap::new(),
            next_socks: 0,
            leaked: Vec::new(),
            copy: None,
            newest_copy: 0,
        }
    }

    /// Take a `ClipboardWrite` chunk: a whole copy is told as `clipboard_write`
    /// (its text when it is UTF-8), one a newer copy cut short as
    /// `clipboard_write_cut`; a late chunk of an older one is dropped.
    fn on_copy_chunk(&mut self, chunk: &ClipboardWrite) {
        if chunk.transfer_id < self.newest_copy {
            return;
        }
        self.newest_copy = chunk.transfer_id;
        match &mut self.copy {
            Some(copy) if copy.first.transfer_id == chunk.transfer_id => {
                copy.data.extend_from_slice(&chunk.data);
                copy.chunks += 1;
            }
            _ => {
                if let Some(cut) = self.copy.take() {
                    emit(json!({
                        "ev": "clipboard_write_cut", "transfer": cut.first.transfer_id,
                        "pane": cut.first.pane_id, "received": cut.data.len(),
                    }));
                }
                self.copy = Some(IncomingCopy {
                    first: ClipboardWrite {
                        data: Bytes::new(),
                        ..chunk.clone()
                    },
                    data: chunk.data.to_vec(),
                    chunks: 1,
                });
            }
        }
        if !chunk.last {
            return;
        }
        let copy = self.copy.take().expect("set above");
        emit(json!({
            "ev": "clipboard_write", "transfer": copy.first.transfer_id,
            "pane": copy.first.pane_id, "mime": copy.first.mime,
            "total_size": copy.first.total_size, "size": copy.data.len(), "chunks": copy.chunks,
            "sha256": crate::transfer::sha256(&copy.data),
            "text": std::str::from_utf8(&copy.data).ok(),
        }));
    }

    pub fn connected(&self) -> bool {
        self.ws.is_some()
    }

    pub fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    pub fn nonce(&mut self) -> u64 {
        self.nonce += 1;
        self.nonce
    }

    /// Connect and say hello; the answer as an event payload.
    pub async fn connect(&mut self, target: Target) -> Result<Value> {
        self.ws = None;
        self.lost_at = None;
        let started = Instant::now();
        let (ws, ack) = open(&target).await?;
        self.install(ws, &ack);
        let client_id = target.client_id.clone();
        self.target = Some(target);
        Ok(json!({
            "server_version": ack.server_version,
            "protocol_version": ack.protocol_version,
            "features": ack.features,
            "server_id": ack.server_id,
            "seq_base": ack.seq_base,
            "fs_max_bytes": ack.fs_max_bytes,
            "clipboard_max_bytes": ack.clipboard_max_bytes,
            "client_id": client_id,
            "ms": ms(started.elapsed()),
        }))
    }

    fn install(&mut self, ws: Ws, ack: &tilr_proto::HelloAck) {
        self.ws = Some(ws);
        self.features.clone_from(&ack.features);
        self.seq_base = ack.seq_base;
        self.seq_n = 0;
        self.generation += 1;
        // Transfer ids count on each connection.
        self.copy = None;
        self.newest_copy = 0;
        self.trace.new_connection(ack.seq_base);
        self.last_rx = Instant::now();
        self.last_pong = Instant::now();
        if let Some(keepalive) = &self.keepalive {
            self.next_ping = Instant::now() + keepalive.interval;
        }
    }

    pub fn set_keepalive(&mut self, keepalive: Option<Keepalive>) {
        if let Some(k) = &keepalive {
            self.next_ping = Instant::now() + k.interval;
            self.last_rx = Instant::now();
            self.last_pong = Instant::now();
        }
        self.keepalive = keepalive;
    }

    fn auto_reconnect(&self) -> bool {
        self.keepalive.as_ref().is_some_and(|k| k.auto_reconnect)
    }

    /// Whether the connection is down for good: lost and not being made
    /// again.
    pub fn down(&self) -> bool {
        self.ws.is_none() && self.lost_at.is_none()
    }

    /// Forget the connection, as when the network goes: the socket is
    /// dropped without a closing handshake.
    pub fn lose(&mut self, reason: &str) {
        self.lose_as(reason, false);
    }

    /// `superseded`: the server closed the connection because a newer one of
    /// this client took over; a client does not reconnect that one.
    fn lose_as(&mut self, reason: &str, superseded: bool) {
        if self.ws.take().is_none() {
            return;
        }
        emit(json!({"ev": "disconnected", "reason": reason, "superseded": superseded}));
        self.forget_streams(reason);
        if self.auto_reconnect() && self.target.is_some() && !superseded {
            self.lost_at = Some((Instant::now(), reason.to_owned()));
            self.attempts = 0;
        }
    }

    /// Close the connection properly, with a WebSocket close.
    pub async fn disconnect(&mut self) {
        self.lost_at = None;
        if let Some(mut ws) = self.ws.take() {
            let _ = tokio::time::timeout(SEND_TIMEOUT, ws.close(None)).await;
        }
        self.forget_streams("disconnected");
    }

    /// Streams end with the connection that carried them, and the screens it
    /// kept up to date are no longer the panes' until a new full frame.
    fn forget_streams(&mut self, reason: &str) {
        for screen in self.screens.values_mut() {
            screen.stale = true;
        }
        self.outbox.clear();
        for (id, stream) in std::mem::take(&mut self.streams) {
            emit(json!({
                "ev": "stream_lost", "stream": id, "reason": reason,
                "bytes_in": stream.bytes_in, "bytes_out": stream.bytes_out,
            }));
        }
        // Transfers do not carry over to another connection.
        for mut upload in std::mem::take(&mut self.uploads) {
            let mut out = upload.outcome(&format!("connection lost: {reason}"));
            out["lost"] = json!(true);
            self.finish(upload.id, upload.background, out);
        }
        for mut download in std::mem::take(&mut self.downloads) {
            let mut out = download.outcome(&format!("connection lost: {reason}"));
            out["lost"] = json!(true);
            self.finish(download.id, download.background, out);
        }
    }

    /// A transfer is over: its outcome waits for `fs_wait` (or the command
    /// that started it); one in the background also says so (`fs_done`).
    pub fn finish(&mut self, id: u64, background: bool, outcome: Value) {
        if background {
            let mut event = outcome.clone();
            event["ev"] = json!("fs_done");
            emit(event);
        }
        self.finished.insert(id, outcome);
    }

    /// Whether some transfer has something to send now, or when it will.
    fn send_due(&self) -> Option<Instant> {
        let uploads = self.uploads.iter().filter_map(Upload::due);
        let now = Instant::now();
        let reads = self.downloads.iter().filter(|d| d.ask_next).map(|_| now);
        uploads.chain(reads).min()
    }

    pub async fn send(&mut self, msg: Msg) -> Result<()> {
        self.send_message(ClientMessage::from(msg)).await
    }

    /// Send an input message named by the next seq of this connection.
    pub async fn send_input(&mut self, msg: Msg) -> Result<u64> {
        self.seq_n += 1;
        let seq = self.seq_base + self.seq_n;
        self.trace.input_sent(seq);
        self.send_message(ClientMessage::with_seq(msg, seq)).await?;
        Ok(seq)
    }

    async fn send_message(&mut self, msg: ClientMessage) -> Result<()> {
        let ws = self.ws.as_mut().ok_or_else(|| anyhow!("not connected"))?;
        let packet = tilr_proto::encode(&msg);
        match tokio::time::timeout(SEND_TIMEOUT, ws.send(Message::Binary(packet))).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => {
                self.lose(&format!("send failed: {err}"));
                bail!("send failed: {err}")
            }
            Err(_) => {
                self.lose("send timed out");
                bail!("send timed out")
            }
        }
    }

    pub async fn subscribe(&mut self, pane: &str) -> Result<()> {
        self.subscribed.insert(pane.to_owned());
        self.screens.insert(pane.to_owned(), Screen::fresh());
        self.send(Msg::Subscribe(Subscribe {
            pane_id: pane.to_owned(),
        }))
        .await
    }

    async fn resubscribe(&mut self) -> Result<()> {
        for pane in self.subscribed.clone() {
            self.subscribe(&pane).await?;
        }
        Ok(())
    }

    /// Drop the connection and make it again, then watch the same panes.
    pub async fn reconnect(&mut self, timeout: Duration) -> Result<Value> {
        let target = self.target.clone().context("never connected")?;
        self.lost_at = None;
        if self.ws.take().is_some() {
            emit(json!({"ev": "disconnected", "reason": "reconnect requested"}));
            self.forget_streams("reconnect requested");
        }
        let started = Instant::now();
        let mut attempts = 0;
        loop {
            attempts += 1;
            let left = timeout.saturating_sub(started.elapsed());
            let attempt = Target {
                timeout: left.min(ATTEMPT_TIMEOUT),
                ..target.clone()
            };
            match open(&attempt).await {
                Ok((ws, ack)) => {
                    self.install(ws, &ack);
                    self.resubscribe().await?;
                    return Ok(json!({
                        "ms": ms(started.elapsed()),
                        "attempts": attempts,
                        "seq_base": ack.seq_base,
                        "server_version": ack.server_version,
                    }));
                }
                Err(err) if started.elapsed() + RETRY_PAUSE >= timeout => {
                    bail!("no connection after {attempts} attempts: {err:#}")
                }
                Err(_) => tokio::time::sleep(RETRY_PAUSE).await,
            }
        }
    }

    /// Send what is waiting, then wait for the next thing to happen, up to
    /// `deadline` (see [`Self::read`]). Not to be cancelled (see
    /// [`Self::flush`]); the command loop calls the two apart.
    pub async fn pump(&mut self, deadline: Instant) -> Option<server_message::Msg> {
        self.flush().await;
        self.read(deadline).await
    }

    /// Send what is waiting: echoes, a local agent's bytes, the next chunk of
    /// each write that is due, the next part of each read. Not to be
    /// cancelled: a chunk is counted as sent once its send returns.
    pub async fn flush(&mut self) {
        if self.ws.is_none() {
            return;
        }
        while let Some(msg) = self.outbox.pop_front() {
            if self.send(msg).await.is_err() {
                return;
            }
        }
        let now = Instant::now();
        let mut n = 0;
        while n < self.uploads.len() {
            if self.uploads[n].due().is_none_or(|due| due > now) {
                n += 1;
                continue;
            }
            let chunk = match self.uploads[n].next_chunk() {
                Ok(chunk) => chunk,
                Err(err) => {
                    let mut upload = self.uploads.remove(n);
                    let out = upload.outcome(&format!("{err:#}"));
                    self.finish(upload.id, upload.background, out);
                    continue;
                }
            };
            let (len, last) = (chunk.data.len(), chunk.last);
            if self.send(Msg::FsWrite(chunk)).await.is_err() {
                // The connection is gone, and its transfers with it.
                return;
            }
            self.uploads[n].sent_chunk(len, last);
            n += 1;
        }
        let mut asks = Vec::new();
        for download in self.downloads.iter_mut().filter(|d| d.ask_next) {
            self.next_request += 1;
            asks.push(download.next_request(self.next_request));
        }
        for ask in asks {
            if self.send(Msg::FsRead(ask)).await.is_err() {
                return;
            }
        }
    }

    /// Wait for the next thing to happen, up to `deadline`: a message from
    /// the server (returned), a keepalive step, a reconnect attempt, a
    /// transfer having something to send. Cancel-safe, but for a ping being
    /// sent when cancelled, which may be lost.
    pub async fn read(&mut self, deadline: Instant) -> Option<server_message::Msg> {
        if self.ws.is_none() {
            if self.lost_at.is_some() {
                self.reconnect_step(deadline).await;
            } else {
                sleep_until(deadline.into()).await;
            }
            return None;
        }
        let wake = self
            .keepalive_due()
            .map_or(deadline, |due| due.min(deadline));
        let wake = self.send_due().map_or(wake, |due| due.min(wake));
        let ws = self.ws.as_mut()?;
        let relay = &mut self.relay_rx;
        let next = tokio::select! {
            message = ws.next() => Next::Message(message),
            Some((stream_id, data)) = relay.recv() => Next::Agent(stream_id, data),
            () = sleep_until(wake.into()) => Next::Wake,
        };
        match next {
            Next::Message(message) => self.receive(message),
            Next::Agent(stream_id, data) => {
                self.on_agent_bytes(stream_id, data);
                None
            }
            Next::Wake => {
                self.keepalive_step().await;
                None
            }
        }
    }

    /// Bytes from a local agent (or its end) for the server.
    fn on_agent_bytes(&mut self, stream_id: u64, data: Option<Bytes>) {
        let Some(stream) = self.streams.get_mut(&stream_id) else {
            return;
        };
        match data {
            Some(data) => {
                stream.bytes_out += data.len() as u64;
                self.outbox
                    .push_back(Msg::StreamData(StreamData { stream_id, data }));
            }
            None => {
                self.streams.remove(&stream_id);
                self.outbox
                    .push_back(Msg::StreamClose(StreamClose { stream_id }));
            }
        }
    }

    fn receive(
        &mut self,
        message: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) -> Option<server_message::Msg> {
        let message = match message {
            Some(Ok(message)) => message,
            Some(Err(err)) => {
                self.lose(&format!("read failed: {err}"));
                return None;
            }
            None => {
                self.lose("connection ended");
                return None;
            }
        };
        self.last_rx = Instant::now();
        let packet = match message {
            Message::Binary(packet) => packet,
            Message::Close(frame) => {
                let superseded = frame
                    .as_ref()
                    .is_some_and(|f| u16::from(f.code) == tilr_proto::close::SUPERSEDED);
                let reason = frame.map(|f| format!("{} {}", f.code, f.reason));
                let reason = format!("server closed {}", reason.unwrap_or_default());
                self.lose_as(reason.trim(), superseded);
                return None;
            }
            _ => return None,
        };
        let msg = match tilr_proto::decode::<ServerMessage>(&packet) {
            Ok(ServerMessage { msg: Some(msg) }) => msg,
            Ok(_) => return None,
            Err(err) => {
                emit(json!({"ev": "protocol_error", "message": err.to_string()}));
                return None;
            }
        };
        match &msg {
            server_message::Msg::Frame(frame) => {
                self.trace.frame(frame, packet.len());
                if self.subscribed.contains(&frame.pane_id) {
                    self.screens
                        .entry(frame.pane_id.clone())
                        .or_insert_with(Screen::fresh)
                        .apply(frame);
                }
            }
            server_message::Msg::Pong(_) => self.last_pong = Instant::now(),
            server_message::Msg::PaneExited(exited) => emit(json!({
                "ev": "pane_exited", "pane": exited.pane_id, "code": exited.exit_code,
            })),
            server_message::Msg::Error(error) => {
                emit(json!({"ev": "server_error", "message": error.message}));
            }
            server_message::Msg::StreamOpen(open) => {
                let to_agent = match &self.agent {
                    AgentMode::Socket(path) => {
                        let (tx, rx) = mpsc::unbounded_channel();
                        tokio::spawn(relay::agent(
                            open.stream_id,
                            path.clone(),
                            rx,
                            self.relay_tx.clone(),
                        ));
                        Some(tx)
                    }
                    _ => None,
                };
                self.streams.insert(
                    open.stream_id,
                    Stream {
                        kind: open.kind.clone(),
                        pane: open.pane_id.clone(),
                        opened_ms: now_ms(),
                        bytes_in: 0,
                        bytes_out: 0,
                        sha_in: Sha256::new(),
                        to_agent,
                    },
                );
                emit(json!({
                    "ev": "stream_open", "stream": open.stream_id, "kind": open.kind,
                    "pane": open.pane_id,
                }));
            }
            server_message::Msg::StreamData(data) => {
                if let Some(stream) = self.streams.get_mut(&data.stream_id) {
                    stream.bytes_in += data.data.len() as u64;
                    stream.sha_in.update(&data.data);
                    match (&self.agent, &stream.to_agent) {
                        (AgentMode::Echo, _) => {
                            stream.bytes_out += data.data.len() as u64;
                            self.outbox.push_back(Msg::StreamData(StreamData {
                                stream_id: data.stream_id,
                                data: data.data.clone(),
                            }));
                        }
                        (_, Some(to_agent)) => {
                            let _ = to_agent.send(data.data.clone());
                        }
                        _ => {}
                    }
                }
            }
            server_message::Msg::StreamClose(close) => {
                if let Some(stream) = self.streams.remove(&close.stream_id) {
                    let mut event = stream.to_json(close.stream_id);
                    event["ev"] = json!("stream_closed");
                    emit(event);
                }
            }
            server_message::Msg::FsAck(ack) => {
                if let Some(n) = self
                    .uploads
                    .iter()
                    .position(|u| u.request_id == ack.request_id)
                {
                    // Done, or refused: the rest of a refused write is not sent.
                    let mut upload = self.uploads.remove(n);
                    let out = upload.outcome(&ack.error);
                    self.finish(upload.id, upload.background, out);
                }
            }
            server_message::Msg::FsData(data) => {
                if let Some(n) = self
                    .downloads
                    .iter()
                    .position(|d| d.request_id == data.request_id && !d.ask_next)
                    && let Progress::Done(out) = self.downloads[n].on_data(data)
                {
                    let download = self.downloads.remove(n);
                    self.finish(download.id, download.background, out);
                }
            }
            server_message::Msg::ClipboardWrite(chunk) => self.on_copy_chunk(chunk),
            server_message::Msg::HostEvent(event) => emit(json!({
                "ev": "host_event", "pane": event.pane_id, "name": event.name,
                "kind": event.kind, "path": event.path, "args": event.args,
            })),
            server_message::Msg::ExecStarted(started) => {
                let outcome = if started.error.is_empty() {
                    Ok(started.exec_id)
                } else {
                    Err(anyhow!("exec refused: {}", started.error))
                };
                self.exec_started.insert(started.request_id, outcome);
            }
            server_message::Msg::ExecOutput(output) => {
                if let Some(run) = self.execs.get_mut(&output.exec_id) {
                    run.output(output);
                }
            }
            server_message::Msg::ExecInputAck(ack) => {
                if let Some(run) = self.execs.get_mut(&ack.exec_id) {
                    run.taken = run.taken.max(ack.taken);
                }
            }
            server_message::Msg::ExecExit(exit) => {
                if let Some(run) = self.execs.get_mut(&exit.exec_id) {
                    let mut value = run.exited(exit);
                    if run.background {
                        value["ev"] = json!("exec_exit");
                        emit(value);
                    }
                }
            }
            _ => {}
        }
        Some(msg)
    }

    fn keepalive_due(&self) -> Option<Instant> {
        let k = self.keepalive.as_ref()?;
        Some(self.next_ping.min(self.alive_at() + k.timeout))
    }

    /// The last sign of life the keepalive counts.
    fn alive_at(&self) -> Instant {
        match &self.keepalive {
            Some(k) if k.pong_only => self.last_pong,
            _ => self.last_rx,
        }
    }

    async fn keepalive_step(&mut self) {
        let Some(k) = &self.keepalive else { return };
        let (interval, timeout, leak) = (k.interval, k.timeout, k.leak_dead);
        let silent = self.alive_at().elapsed();
        if silent >= timeout {
            emit(json!({
                "ev": "dead", "silent_ms": ms(silent),
                "rx_silent_ms": ms(self.last_rx.elapsed()), "leaked": leak,
            }));
            if leak && let Some(ws) = self.ws.take() {
                // lose() below then finds no connection: say it here.
                self.leaked.push(ws);
                emit(json!({"ev": "disconnected", "reason": "keepalive timeout (left open)"}));
                self.forget_streams("keepalive timeout");
                if self.auto_reconnect() && self.target.is_some() {
                    self.lost_at = Some((Instant::now(), "keepalive timeout".into()));
                    self.attempts = 0;
                }
                return;
            }
            self.lose("keepalive timeout");
            return;
        }
        if Instant::now() >= self.next_ping {
            self.next_ping = Instant::now() + interval;
            let nonce = self.nonce();
            let _ = self.send(Msg::Ping(tilr_proto::Ping { nonce })).await;
        }
    }

    async fn reconnect_step(&mut self, deadline: Instant) {
        let (Some((lost, reason)), Some(target), Some(k)) = (
            self.lost_at.clone(),
            self.target.clone(),
            self.keepalive.as_ref(),
        ) else {
            return;
        };
        let left = k.reconnect_timeout.saturating_sub(lost.elapsed());
        if left.is_zero() {
            emit(json!({
                "ev": "reconnect_failed", "attempts": self.attempts, "ms": ms(lost.elapsed()),
            }));
            self.lost_at = None;
            return;
        }
        let budget = left
            .min(k.attempt_timeout)
            .min(deadline.saturating_duration_since(Instant::now()));
        if budget.is_zero() {
            return;
        }
        self.attempts += 1;
        let attempt = Target {
            timeout: budget,
            ..target
        };
        match open(&attempt).await {
            Ok((ws, ack)) => {
                self.install(ws, &ack);
                self.lost_at = None;
                let resubscribed = self.resubscribe().await;
                emit(json!({
                    "ev": "reconnected",
                    "ms": ms(lost.elapsed()),
                    "attempts": self.attempts,
                    "reason": reason,
                    "resubscribed": resubscribed.is_ok(),
                    "server_version": ack.server_version,
                }));
            }
            Err(_) => {
                let pause = (Instant::now() + RETRY_PAUSE).min(deadline);
                sleep_until(pause.into()).await;
            }
        }
    }
}

/// Open a WebSocket to `url`, with the origin header if any.
pub async fn dial(url: &str, origin: Option<&str>) -> Result<Ws> {
    let mut request = url.into_client_request()?;
    if let Some(origin) = origin {
        request.headers_mut().insert("origin", origin.parse()?);
    }
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connect to {url}"))?;
    if let MaybeTlsStream::Plain(stream) = ws.get_ref() {
        let _ = stream.set_nodelay(true);
    }
    Ok(ws)
}

/// Open a WebSocket to the target and exchange Hello / HelloAck.
async fn open(target: &Target) -> Result<(Ws, tilr_proto::HelloAck)> {
    let attempt = async {
        let mut ws = dial(&target.url, target.origin.as_deref()).await?;
        let hello = ClientMessage::from(Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "tilr-driver".into(),
            features: target.features.clone(),
            token: target.token.clone(),
            client_id: target.client_id.clone(),
        }));
        ws.send(Message::Binary(tilr_proto::encode(&hello))).await?;
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(packet))) => {
                    match tilr_proto::decode::<ServerMessage>(&packet)?.msg {
                        Some(server_message::Msg::HelloAck(ack)) => return Ok((ws, ack)),
                        Some(server_message::Msg::Error(error)) => {
                            bail!("server refused: {}", error.message)
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    bail!("server closed during hello: {frame:?}")
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => bail!("read during hello: {err}"),
                None => bail!("connection ended during hello"),
            }
        }
    };
    tokio::time::timeout(target.timeout, attempt)
        .await
        .map_err(|_| anyhow!("no hello ack within {} ms", target.timeout.as_millis()))?
}

pub fn ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10_000.0).round() / 10.0
}
