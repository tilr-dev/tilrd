//! Scripted tilr client for black-box tests.
//!
//! Reads one JSON command per line on stdin and writes one JSON event per
//! line on stdout. Every event has `ev` and `t` (milliseconds since the
//! driver started, monotonic); the answer to a command also has its `id` and
//! `cmd`. A command that fails is answered with `{"ev": "error", "message"}`:
//! the driver itself never stops on an error. Unsolicited events:
//! `disconnected`, `dead` (keepalive), `reconnected`, `reconnect_failed`,
//! `pane_exited`, `server_error`, `protocol_error`.
//!
//! ```text
//! {"id":1,"cmd":"connect","url":"ws://127.0.0.1:7890","token":"..."}
//! {"id":2,"cmd":"create_pane","cols":80,"rows":24}
//! {"id":3,"cmd":"input","pane":"<id>","text":"echo hi\r"}
//! {"id":4,"cmd":"wait_for","pane":"<id>","regex":"^hi$","timeout_ms":5000}
//! ```
//!
//! Commands: connect, disconnect, drop, reconnect, keepalive, create_pane,
//! close_pane, list, subscribe, unsubscribe, input, paste, key, resize,
//! screen, wait_for, ping, sleep, quit. See itests/README.md.

mod keys;
mod screen;
mod session;

use std::io::Write;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use regex::Regex;
use serde_json::{Map, Value, json};
use tilr_proto::{
    ClosePane, CreatePane, ListPanes, Paste, RawInput, Resize, Unsubscribe, client_message::Msg,
    server_message,
};
use tokio::io::AsyncBufReadExt;

use session::{Keepalive, Session, Target, ms};

static START: OnceLock<Instant> = OnceLock::new();

/// Write an event line, stamped with the time.
pub fn emit(mut event: Value) {
    let t = ms(START.get_or_init(Instant::now).elapsed());
    if let Value::Object(map) = &mut event {
        map.insert("t".into(), json!(t));
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{event}");
    let _ = out.flush();
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    START.get_or_init(Instant::now);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut session = Session::new();
    emit(json!({"ev": "ready", "version": env!("CARGO_PKG_VERSION")}));
    loop {
        // Between commands the connection keeps being read: screens stay
        // current, keepalive and reconnecting go on.
        let line = tokio::select! {
            line = rx.recv() => line,
            _ = session.pump(Instant::now() + Duration::from_secs(3600)) => continue,
        };
        let Some(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let command: Value = match serde_json::from_str(&line) {
            Ok(command) => command,
            Err(err) => {
                emit(
                    json!({"ev": "error", "message": format!("bad command: {err}"), "line": line}),
                );
                continue;
            }
        };
        let id = command.get("id").cloned().unwrap_or(Value::Null);
        let name = command
            .get("cmd")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if name == "quit" {
            session.disconnect().await;
            emit(json!({"ev": "bye", "id": id, "cmd": name}));
            break;
        }
        let started = Instant::now();
        let mut reply = match run(&mut session, &name, &command).await {
            Ok(Value::Object(map)) => Value::Object(map),
            Ok(other) => json!({"ev": "ok", "value": other}),
            Err(err) => json!({"ev": "error", "message": format!("{err:#}")}),
        };
        if let Value::Object(map) = &mut reply {
            map.entry("ev").or_insert(json!("ok"));
            map.insert("id".into(), id);
            map.insert("cmd".into(), json!(name));
            map.entry("took_ms").or_insert(json!(ms(started.elapsed())));
        }
        emit(reply);
    }
}

async fn run(session: &mut Session, name: &str, c: &Value) -> Result<Value> {
    match name {
        "connect" => {
            let target = Target {
                url: string(c, "url")?,
                token: c
                    .get("token")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| std::env::var("TILR_TOKEN").ok())
                    .unwrap_or_default(),
                origin: opt_string(c, "origin"),
                features: c
                    .get("features")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["ping".into()]),
                timeout: millis(c, "timeout_ms", 10_000),
            };
            let mut ack = session.connect(target).await?;
            ack["ev"] = json!("connected");
            Ok(ack)
        }
        "disconnect" => {
            session.disconnect().await;
            Ok(json!({}))
        }
        "drop" => {
            session.lose("dropped by the test");
            Ok(json!({}))
        }
        "reconnect" => {
            let mut done = session.reconnect(millis(c, "timeout_ms", 20_000)).await?;
            done["ev"] = json!("reconnected");
            Ok(done)
        }
        "keepalive" => {
            let interval = millis(c, "interval_ms", 0);
            session.set_keepalive((!interval.is_zero()).then(|| {
                Keepalive {
                    interval,
                    timeout: millis(c, "timeout_ms", 3 * interval.as_millis() as u64),
                    auto_reconnect: c
                        .get("auto_reconnect")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    reconnect_timeout: millis(c, "reconnect_timeout_ms", 60_000),
                }
            }));
            Ok(json!({}))
        }
        "create_pane" => {
            let request_id = session.request_id();
            let command = c
                .get("command")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            session
                .send(Msg::CreatePane(CreatePane {
                    cols: number(c, "cols", 80) as u32,
                    rows: number(c, "rows", 24) as u32,
                    command,
                    cwd: opt_string(c, "cwd").unwrap_or_default(),
                    request_id,
                }))
                .await?;
            let pane = wait_msg(session, millis(c, "timeout_ms", 10_000), |msg| match msg {
                server_message::Msg::PaneCreated(created) if created.request_id == request_id => {
                    Some(Ok(created.pane.clone().unwrap_or_default()))
                }
                server_message::Msg::Error(error) => {
                    Some(Err(anyhow::anyhow!("{}", error.message)))
                }
                _ => None,
            })
            .await?;
            if c.get("subscribe").and_then(Value::as_bool).unwrap_or(true) {
                session.subscribe(&pane.pane_id).await?;
            }
            Ok(json!({"ev": "pane_created", "pane": pane.pane_id, "info": pane_json(&pane)}))
        }
        "close_pane" => {
            let pane = string(c, "pane")?;
            session.subscribed.remove(&pane);
            session
                .send(Msg::ClosePane(ClosePane { pane_id: pane }))
                .await?;
            Ok(json!({}))
        }
        "list" => {
            let request_id = session.request_id();
            session
                .send(Msg::ListPanes(ListPanes { request_id }))
                .await?;
            let panes = wait_msg(session, millis(c, "timeout_ms", 10_000), |msg| match msg {
                server_message::Msg::PaneList(list) if list.request_id == request_id => {
                    Some(Ok(list.panes.iter().map(pane_json).collect::<Vec<_>>()))
                }
                _ => None,
            })
            .await?;
            Ok(json!({"ev": "panes", "panes": panes}))
        }
        "subscribe" => {
            session.subscribe(&string(c, "pane")?).await?;
            Ok(json!({}))
        }
        "unsubscribe" => {
            let pane = string(c, "pane")?;
            session.subscribed.remove(&pane);
            session.screens.remove(&pane);
            session
                .send(Msg::Unsubscribe(Unsubscribe { pane_id: pane }))
                .await?;
            Ok(json!({}))
        }
        "input" => {
            let msg = Msg::Raw(RawInput {
                pane_id: string(c, "pane")?,
                data: Bytes::from(string(c, "text")?.into_bytes()),
            });
            let seq = session.send_input(msg).await?;
            Ok(json!({"seq": seq}))
        }
        "paste" => {
            let msg = Msg::Paste(Paste {
                pane_id: string(c, "pane")?,
                text: string(c, "text")?,
            });
            let seq = session.send_input(msg).await?;
            Ok(json!({"seq": seq}))
        }
        "key" => {
            let pane = string(c, "pane")?;
            let mut seqs = Vec::new();
            for spec in string(c, "key")?.split_whitespace() {
                seqs.push(
                    session
                        .send_input(Msg::Key(keys::parse(&pane, spec)?))
                        .await?,
                );
            }
            Ok(json!({"seq": seqs.last()}))
        }
        "resize" => {
            let msg = Msg::Resize(Resize {
                pane_id: string(c, "pane")?,
                cols: number(c, "cols", 80) as u32,
                rows: number(c, "rows", 24) as u32,
                ..Default::default()
            });
            let seq = session.send_input(msg).await?;
            Ok(json!({"seq": seq}))
        }
        "screen" => {
            let pane = string(c, "pane")?;
            if !session.subscribed.contains(&pane) {
                bail!("not subscribed to {pane}");
            }
            let deadline = Instant::now() + millis(c, "timeout_ms", 5_000);
            while session.screens.get(&pane).is_none_or(|s| s.stale) {
                if Instant::now() >= deadline {
                    bail!("no full frame of {pane} yet");
                }
                session.pump(deadline).await;
            }
            let mut out = session.screens[&pane].to_json();
            out["ev"] = json!("screen");
            out["pane"] = json!(pane);
            Ok(out)
        }
        "wait_for" => {
            let pane = string(c, "pane")?;
            let regex = Regex::new(&string(c, "regex")?).context("bad regex")?;
            if !session.subscribed.contains(&pane) {
                bail!("not subscribed to {pane}");
            }
            let started = Instant::now();
            let deadline = started + millis(c, "timeout_ms", 10_000);
            loop {
                if let Some(screen) = session.screens.get(&pane).filter(|s| !s.stale) {
                    let text = screen.text();
                    if let Some(found) = regex.captures(&text) {
                        let groups: Vec<Value> = found
                            .iter()
                            .map(|g| g.map_or(Value::Null, |g| json!(g.as_str())))
                            .collect();
                        return Ok(json!({
                            "ev": "matched", "match": groups[0], "groups": groups,
                            "ms": ms(started.elapsed()),
                        }));
                    }
                }
                if Instant::now() >= deadline {
                    let text = session
                        .screens
                        .get(&pane)
                        .map(|s| s.text())
                        .unwrap_or_default();
                    bail!("timed out waiting for /{regex}/ in {pane}; screen:\n{text}");
                }
                if session.down() {
                    bail!("disconnected while waiting for /{regex}/");
                }
                session.pump(deadline).await;
            }
        }
        "ping" => {
            let nonce = session.nonce();
            let started = Instant::now();
            session.send(Msg::Ping(tilr_proto::Ping { nonce })).await?;
            wait_msg(session, millis(c, "timeout_ms", 5_000), |msg| match msg {
                server_message::Msg::Pong(pong) if pong.nonce == nonce => Some(Ok(())),
                _ => None,
            })
            .await?;
            Ok(json!({"ev": "pong", "rtt_ms": ms(started.elapsed())}))
        }
        "sleep" => {
            let deadline = Instant::now() + millis(c, "ms", 0);
            while Instant::now() < deadline {
                session.pump(deadline).await;
            }
            Ok(json!({}))
        }
        "status" => Ok(json!({
            "connected": session.connected(),
            "subscribed": session.subscribed.iter().collect::<Vec<_>>(),
        })),
        "" => bail!("command without \"cmd\""),
        other => bail!("unknown command {other:?}"),
    }
}

/// Read server messages until `pick` takes one, the connection is down for
/// good or the time is up.
async fn wait_msg<T>(
    session: &mut Session,
    timeout: Duration,
    mut pick: impl FnMut(&server_message::Msg) -> Option<Result<T>>,
) -> Result<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            bail!("no answer within {} ms", timeout.as_millis());
        }
        if session.down() {
            bail!("not connected");
        }
        if let Some(msg) = session.pump(deadline).await
            && let Some(picked) = pick(&msg)
        {
            return picked;
        }
    }
}

fn pane_json(pane: &tilr_proto::PaneInfo) -> Value {
    let mut map = Map::new();
    map.insert("pane".into(), json!(pane.pane_id));
    map.insert("cols".into(), json!(pane.cols));
    map.insert("rows".into(), json!(pane.rows));
    map.insert("command".into(), json!(pane.command));
    map.insert("title".into(), json!(pane.title));
    map.insert("cwd".into(), json!(pane.cwd));
    map.insert("viewers".into(), json!(pane.viewers));
    map.insert("pid".into(), json!(pane.pid));
    map.insert("created_at_ms".into(), json!(pane.created_at_ms));
    Value::Object(map)
}

fn string(c: &Value, key: &str) -> Result<String> {
    opt_string(c, key).with_context(|| format!("missing \"{key}\""))
}

fn opt_string(c: &Value, key: &str) -> Option<String> {
    c.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn number(c: &Value, key: &str, default: u64) -> u64 {
    c.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn millis(c: &Value, key: &str, default: u64) -> Duration {
    Duration::from_millis(number(c, key, default))
}
