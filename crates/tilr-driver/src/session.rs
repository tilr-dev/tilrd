//! One client connection to a tilr server and what it knows: the screens of
//! the panes it watches, keepalive and reconnecting, the streams the server
//! opened to it, and how long its input and frames took.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tilr_proto::{
    ClientMessage, Hello, PROTOCOL_VERSION, ServerMessage, StreamData, Subscribe,
    client_message::Msg, server_message,
};
use tokio::net::TcpStream;
use tokio::time::sleep_until;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::screen::Screen;
use crate::socks::Socks;
use crate::trace::Trace;
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
    /// Echo what a program sends on a stream back to it (a toy agent).
    pub agent_echo: bool,
    /// Messages to send before the next read (the echoes).
    outbox: VecDeque<Msg>,
    pub socks: BTreeMap<u64, Socks>,
    pub next_socks: u64,
    /// Connections given up on but left open (Keepalive.leak_dead).
    leaked: Vec<Ws>,
}

impl Session {
    pub fn new() -> Self {
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
            agent_echo: true,
            outbox: VecDeque::new(),
            socks: BTreeMap::new(),
            next_socks: 0,
            leaked: Vec::new(),
        }
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
            "client_id": client_id,
            "ms": ms(started.elapsed()),
        }))
    }

    fn install(&mut self, ws: Ws, ack: &tilr_proto::HelloAck) {
        self.ws = Some(ws);
        self.seq_base = ack.seq_base;
        self.seq_n = 0;
        self.generation += 1;
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

    /// Wait for the next thing to happen, up to `deadline`: a message from
    /// the server (returned), a keepalive step, a reconnect attempt.
    /// Cancel-safe, but for an echo being sent when cancelled, which may be
    /// lost (never sent twice).
    pub async fn pump(&mut self, deadline: Instant) -> Option<server_message::Msg> {
        if self.ws.is_none() {
            if self.lost_at.is_some() {
                self.reconnect_step(deadline).await;
            } else {
                sleep_until(deadline.into()).await;
            }
            return None;
        }
        while let Some(msg) = self.outbox.pop_front() {
            if self.send(msg).await.is_err() {
                return None;
            }
        }
        let wake = self
            .keepalive_due()
            .map_or(deadline, |due| due.min(deadline));
        let ws = self.ws.as_mut()?;
        let next = tokio::select! {
            message = ws.next() => Some(message),
            () = sleep_until(wake.into()) => None,
        };
        match next {
            Some(message) => self.receive(message),
            None => {
                self.keepalive_step().await;
                None
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
            Ok(decoded) => decoded.msg?,
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
                self.streams.insert(
                    open.stream_id,
                    Stream {
                        kind: open.kind.clone(),
                        pane: open.pane_id.clone(),
                        opened_ms: now_ms(),
                        bytes_in: 0,
                        bytes_out: 0,
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
                    if self.agent_echo {
                        stream.bytes_out += data.data.len() as u64;
                        self.outbox.push_back(Msg::StreamData(StreamData {
                            stream_id: data.stream_id,
                            data: data.data.clone(),
                        }));
                    }
                }
            }
            server_message::Msg::StreamClose(close) => {
                if let Some(stream) = self.streams.remove(&close.stream_id) {
                    emit(json!({
                        "ev": "stream_closed", "stream": close.stream_id,
                        "bytes_in": stream.bytes_in, "bytes_out": stream.bytes_out,
                    }));
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
