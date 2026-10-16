//! One client connection to a tilr server and what it knows: the screens of
//! the panes it watches, keepalive and reconnecting.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tilr_proto::{
    ClientMessage, Hello, PROTOCOL_VERSION, ServerMessage, Subscribe, client_message::Msg,
    server_message,
};
use tokio::net::TcpStream;
use tokio::time::sleep_until;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::emit;
use crate::screen::Screen;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

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
}

pub struct Session {
    ws: Option<Ws>,
    target: Option<Target>,
    pub screens: HashMap<String, Screen>,
    pub subscribed: BTreeSet<String>,
    seq_base: u64,
    seq_n: u64,
    last_rx: Instant,
    keepalive: Option<Keepalive>,
    next_ping: Instant,
    nonce: u64,
    /// The connection was lost and is to be made again (auto reconnect).
    lost_at: Option<(Instant, String)>,
    attempts: u32,
    next_request: u64,
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
            keepalive: None,
            next_ping: Instant::now(),
            nonce: 0,
            lost_at: None,
            attempts: 0,
            next_request: 0,
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
        self.target = Some(target);
        Ok(json!({
            "server_version": ack.server_version,
            "protocol_version": ack.protocol_version,
            "features": ack.features,
            "server_id": ack.server_id,
            "seq_base": ack.seq_base,
            "ms": ms(started.elapsed()),
        }))
    }

    fn install(&mut self, ws: Ws, ack: &tilr_proto::HelloAck) {
        self.ws = Some(ws);
        self.seq_base = ack.seq_base;
        self.seq_n = 0;
        self.last_rx = Instant::now();
        if let Some(keepalive) = &self.keepalive {
            self.next_ping = Instant::now() + keepalive.interval;
        }
    }

    pub fn set_keepalive(&mut self, keepalive: Option<Keepalive>) {
        if let Some(k) = &keepalive {
            self.next_ping = Instant::now() + k.interval;
            self.last_rx = Instant::now();
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
        if self.ws.take().is_none() {
            return;
        }
        emit(json!({"ev": "disconnected", "reason": reason}));
        if self.auto_reconnect() && self.target.is_some() {
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
    }

    pub async fn send(&mut self, msg: Msg) -> Result<()> {
        self.send_message(ClientMessage::from(msg)).await
    }

    /// Send an input message named by the next seq of this connection.
    pub async fn send_input(&mut self, msg: Msg) -> Result<u64> {
        self.seq_n += 1;
        let seq = self.seq_base + self.seq_n;
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
    /// Cancel-safe.
    pub async fn pump(&mut self, deadline: Instant) -> Option<server_message::Msg> {
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
                let reason = frame.map(|f| format!("{} {}", f.code, f.reason));
                self.lose(format!("server closed {}", reason.unwrap_or_default()).trim());
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
                if self.subscribed.contains(&frame.pane_id) {
                    self.screens
                        .entry(frame.pane_id.clone())
                        .or_insert_with(Screen::fresh)
                        .apply(frame);
                }
            }
            server_message::Msg::PaneExited(exited) => emit(json!({
                "ev": "pane_exited", "pane": exited.pane_id, "code": exited.exit_code,
            })),
            server_message::Msg::Error(error) => {
                emit(json!({"ev": "server_error", "message": error.message}));
            }
            _ => {}
        }
        Some(msg)
    }

    fn keepalive_due(&self) -> Option<Instant> {
        let k = self.keepalive.as_ref()?;
        Some(self.next_ping.min(self.last_rx + k.timeout))
    }

    async fn keepalive_step(&mut self) {
        let Some(k) = &self.keepalive else { return };
        let (interval, timeout) = (k.interval, k.timeout);
        let silent = self.last_rx.elapsed();
        if silent >= timeout {
            emit(json!({"ev": "dead", "silent_ms": ms(silent)}));
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
            .min(ATTEMPT_TIMEOUT)
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

/// Open a WebSocket to the target and exchange Hello / HelloAck.
async fn open(target: &Target) -> Result<(Ws, tilr_proto::HelloAck)> {
    let attempt = async {
        let mut request = target.url.as_str().into_client_request()?;
        if let Some(origin) = &target.origin {
            request.headers_mut().insert("origin", origin.parse()?);
        }
        let (mut ws, _) = tokio_tungstenite::connect_async(request)
            .await
            .with_context(|| format!("connect to {}", target.url))?;
        if let MaybeTlsStream::Plain(stream) = ws.get_ref() {
            let _ = stream.set_nodelay(true);
        }
        let hello = ClientMessage::from(Msg::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "tilr-driver".into(),
            features: target.features.clone(),
            token: target.token.clone(),
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
