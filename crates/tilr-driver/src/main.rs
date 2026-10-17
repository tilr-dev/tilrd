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
//! close_pane, list, subscribe, unsubscribe, input, paste, key, mouse,
//! resize, screen, wait_for, ping, sleep, fs_write, fs_read, fs_wait,
//! fs_list, fs_stat, clipboard_put, agent, streams, stream_send,
//! stream_close, socks_open, socks_send, socks_recv, socks_close,
//! socks_listen, socks_stats, exec, exec_input, exec_signal, exec_cancel,
//! exec_wait, latency, frames, quit. See itests/README.md.

mod exec;
mod keys;
mod relay;
mod screen;
mod session;
mod socks;
mod trace;
mod transfer;

use std::io::Write;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use regex::Regex;
use serde_json::{Map, Value, json};
use tilr_proto::{
    ClipboardPut, ClosePane, CreatePane, FsList, FsStat, ListPanes, MouseInput, Paste, RawInput,
    Resize, StreamClose, StreamData, Unsubscribe, client_message::Msg, server_message,
};
use tokio::io::AsyncBufReadExt;

use session::{AgentMode, Keepalive, Session, Target, ms};
use transfer::{Download, DownloadSpec, Source, Upload, UploadSpec};

static START: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the driver started.
pub fn now_ms() -> f64 {
    ms(START.get_or_init(Instant::now).elapsed())
}

/// Write an event line, stamped with the time.
pub fn emit(mut event: Value) {
    let t = now_ms();
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
        session.flush().await;
        let line = tokio::select! {
            line = rx.recv() => line,
            _ = session.read(Instant::now() + Duration::from_secs(3600)) => continue,
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
                // One per driver, kept across its reconnects, as a client
                // instance keeps it; a case may name another (a second tab of
                // the same client, a spoof).
                client_id: opt_string(c, "client_id").unwrap_or_else(|| client_id().to_owned()),
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
                    attempt_timeout: millis(c, "attempt_timeout_ms", 3_000),
                    pong_only: opt_string(c, "mode").as_deref() == Some("pong"),
                    leak_dead: opt_string(c, "abandon").as_deref() == Some("leak"),
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
        "mouse" => {
            let action = match opt_string(c, "action").as_deref().unwrap_or("press") {
                "press" => tilr_proto::MouseAction::Press,
                "release" => tilr_proto::MouseAction::Release,
                "motion" => tilr_proto::MouseAction::Motion,
                other => bail!("unknown mouse action {other:?}"),
            };
            let button = match opt_string(c, "button").as_deref().unwrap_or("left") {
                "none" => tilr_proto::MouseButton::None,
                "left" => tilr_proto::MouseButton::Left,
                "right" => tilr_proto::MouseButton::Right,
                "middle" => tilr_proto::MouseButton::Middle,
                "wheel_up" => tilr_proto::MouseButton::WheelUp,
                "wheel_down" => tilr_proto::MouseButton::WheelDown,
                other => bail!("unknown mouse button {other:?}"),
            };
            let msg = Msg::Mouse(MouseInput {
                pane_id: string(c, "pane")?,
                action: action as i32,
                button: button as i32,
                mods: number(c, "mods", 0) as u32,
                x: number(c, "x", 0) as u32,
                y: number(c, "y", 0) as u32,
            });
            let seq = session.send_input(msg).await?;
            Ok(json!({"seq": seq}))
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
        "fs_write" => fs_write(session, c).await,
        "fs_read" => fs_read(session, c).await,
        "fs_wait" => {
            let id = number(c, "transfer", 0);
            let mut out = wait_transfer(session, id, millis(c, "timeout_ms", 60_000)).await?;
            out["ev"] = json!("fs_done");
            Ok(out)
        }
        "fs_list" => {
            let request_id = session.request_id();
            let path = string(c, "path")?;
            session
                .send(Msg::FsList(FsList { request_id, path }))
                .await?;
            wait_msg(session, millis(c, "timeout_ms", 10_000), |msg| match msg {
                server_message::Msg::FsEntries(list) if list.request_id == request_id => {
                    let entries: Vec<Value> = list.entries.iter().map(entry_json).collect();
                    Some(Ok(json!({
                        "ev": "fs_entries", "path": list.path, "entries": entries, "error": list.error,
                    })))
                }
                _ => None,
            })
            .await
        }
        "fs_stat" => {
            let request_id = session.request_id();
            let path = string(c, "path")?;
            session
                .send(Msg::FsStat(FsStat { request_id, path }))
                .await?;
            wait_msg(session, millis(c, "timeout_ms", 10_000), |msg| match msg {
                server_message::Msg::FsInfo(info) if info.request_id == request_id => {
                    Some(Ok(json!({
                        "ev": "fs_info", "path": info.path, "error": info.error,
                        "entry": info.entry.as_ref().map(entry_json),
                    })))
                }
                _ => None,
            })
            .await
        }
        "clipboard_put" => clipboard_put(session, c).await,
        "agent" => {
            if let Some(mode) = opt_string(c, "mode") {
                session.agent = match mode.as_str() {
                    "echo" => AgentMode::Echo,
                    "sink" => AgentMode::Sink,
                    "socket" => AgentMode::Socket(string(c, "socket")?.into()),
                    other => bail!("unknown agent mode {other:?}"),
                };
            } else if let Some(echo) = c.get("echo").and_then(Value::as_bool) {
                session.agent = if echo {
                    AgentMode::Echo
                } else {
                    AgentMode::Sink
                };
            }
            Ok(json!({"mode": format!("{:?}", session.agent)}))
        }
        "streams" => {
            let open: Vec<Value> = session
                .streams
                .iter()
                .map(|(id, s)| s.to_json(*id))
                .collect();
            Ok(json!({"ev": "streams", "streams": open}))
        }
        "stream_send" => {
            let stream_id = number(c, "stream", 0);
            let data = content(c)?;
            let started = Instant::now();
            for piece in data.chunks(64 * 1024) {
                session
                    .send(Msg::StreamData(StreamData {
                        stream_id,
                        data: Bytes::copy_from_slice(piece),
                    }))
                    .await?;
            }
            if let Some(stream) = session.streams.get_mut(&stream_id) {
                stream.bytes_out += data.len() as u64;
            }
            Ok(
                json!({"sent": data.len(), "sha256": transfer::sha256(&data), "ms": ms(started.elapsed())}),
            )
        }
        "stream_close" => {
            let stream_id = number(c, "stream", 0);
            session.streams.remove(&stream_id);
            session
                .send(Msg::StreamClose(StreamClose { stream_id }))
                .await?;
            Ok(json!({}))
        }
        "socks_open" => {
            let target = session.target.clone().context("never connected")?;
            let url = match opt_string(c, "url") {
                Some(url) => url,
                None => format!(
                    "{}/socks?token={}",
                    target.url.trim_end_matches('/'),
                    target.token
                ),
            };
            let started = Instant::now();
            let socks = socks::Socks::open(
                &url,
                &opt_string(c, "host").unwrap_or_else(|| "127.0.0.1".into()),
                number(c, "port", 0) as u16,
                millis(c, "timeout_ms", 10_000),
            )
            .await?;
            session.next_socks += 1;
            let id = session.next_socks;
            session.socks.insert(id, socks);
            Ok(json!({"ev": "socks_open", "socks": id, "ms": ms(started.elapsed())}))
        }
        "socks_send" => {
            let id = number(c, "socks", 0);
            let data = content(c)?;
            let socks = session
                .socks
                .get_mut(&id)
                .context("no such SOCKS session")?;
            socks.send(&data).await?;
            Ok(json!({"sent": data.len(), "bytes_out": socks.bytes_out}))
        }
        "socks_recv" => {
            let id = number(c, "socks", 0);
            let socks = session
                .socks
                .get_mut(&id)
                .context("no such SOCKS session")?;
            let mut out = socks
                .recv(
                    number(c, "bytes", 1) as usize,
                    millis(c, "timeout_ms", 5_000),
                )
                .await;
            out["ev"] = json!("socks_data");
            Ok(out)
        }
        "socks_listen" => {
            let target = session.target.clone().context("never connected")?;
            let url = match opt_string(c, "url") {
                Some(url) => url,
                None => format!(
                    "{}/socks?token={}",
                    target.url.trim_end_matches('/'),
                    target.token
                ),
            };
            let addr = opt_string(c, "addr").unwrap_or_else(|| "127.0.0.1:0".into());
            let port = relay::socks_listen(&addr, url, Arc::clone(&session.socks_stats)).await?;
            Ok(json!({"ev": "socks_listening", "port": port}))
        }
        "socks_stats" => {
            let mut out = session.socks_stats.to_json();
            out["ev"] = json!("socks_stats");
            Ok(out)
        }
        "exec" => exec_start(session, c).await,
        "exec_input" => {
            let exec_id = number(c, "exec", 0);
            let data =
                if c.get("text").is_some() || c.get("size").is_some() || c.get("from").is_some() {
                    content(c)?
                } else {
                    Vec::new()
                };
            session
                .send_draft(exec::Client::Input(exec::ExecInput {
                    exec_id,
                    data: Bytes::from(data),
                    eof: flag(c, "eof"),
                }))
                .await?;
            Ok(json!({}))
        }
        "exec_signal" => {
            let exec_id = number(c, "exec", 0);
            let signal = c.get("signal").and_then(Value::as_i64).unwrap_or(15) as i32;
            session
                .send_draft(exec::Client::Signal(exec::ExecSignal { exec_id, signal }))
                .await?;
            Ok(json!({}))
        }
        "exec_cancel" => {
            let exec_id = number(c, "exec", 0);
            session
                .send_draft(exec::Client::Cancel(exec::ExecCancel { exec_id }))
                .await?;
            Ok(json!({}))
        }
        "exec_wait" => {
            let mut out = wait_exec(
                session,
                number(c, "exec", 0),
                millis(c, "timeout_ms", 30_000),
            )
            .await?;
            out["ev"] = json!("exec_exit");
            Ok(out)
        }
        "socks_close" => {
            let id = number(c, "socks", 0);
            if let Some(socks) = session.socks.remove(&id) {
                socks.close().await;
            }
            Ok(json!({}))
        }
        "latency" => {
            let mut out = session.trace.latency(flag(c, "reset"));
            out["ev"] = json!("latency");
            Ok(out)
        }
        "frames" => {
            let pane = opt_string(c, "pane");
            let mut out =
                session
                    .trace
                    .frames(number(c, "since", 0), pane.as_deref(), flag(c, "reset"));
            out["ev"] = json!("frames");
            Ok(out)
        }
        "status" => Ok(json!({
            "connected": session.connected(),
            "subscribed": session.subscribed.iter().collect::<Vec<_>>(),
        })),
        "" => bail!("command without \"cmd\""),
        other => bail!("unknown command {other:?}"),
    }
}

/// Replace a file on the server's machine (FsWrite) in chunks, `pause_ms`
/// apart, and wait for the FsAck. Fails if the connection is lost on the
/// way (a reconnect does not carry the transfer over). `background`: answer
/// at once with the transfer's id; `fs_done` says how it ended.
async fn fs_write(session: &mut Session, c: &Value) -> Result<Value> {
    let (source, total) = match opt_string(c, "from") {
        Some(from) => {
            let file = std::fs::File::open(&from).with_context(|| format!("open {from}"))?;
            let size = file.metadata()?.len();
            (Source::File(file), size)
        }
        None => {
            let data = content(c)?;
            let size = data.len() as u64;
            (Source::Bytes(data), size)
        }
    };
    let spec = UploadSpec {
        path: string(c, "path")?,
        source,
        total,
        chunk: number(c, "chunk", 64 * 1024) as usize,
        pause: millis(c, "pause_ms", 0),
        create: flag_or(c, "create", true),
        truncate: flag_or(c, "truncate", true),
    };
    if !session.connected() {
        bail!("not connected");
    }
    session.next_transfer += 1;
    let id = session.next_transfer;
    let request_id = session.request_id();
    let background = flag(c, "background");
    session
        .uploads
        .push(Upload::new(id, request_id, spec, background));
    if background {
        return Ok(json!({"ev": "fs_started", "transfer": id}));
    }
    let deadline = Instant::now() + millis(c, "timeout_ms", 30_000);
    if !flag_or(c, "wait", true) {
        while let Some(upload) = session.uploads.iter().find(|u| u.id == id) {
            if upload.all_sent {
                return Ok(json!({"sent": upload.sent, "chunks": upload.chunks}));
            }
            if Instant::now() >= deadline {
                bail!(
                    "not all sent within the timeout ({} bytes sent)",
                    upload.sent
                );
            }
            session.pump(deadline).await;
        }
    }
    let mut out = wait_transfer(
        session,
        id,
        deadline.saturating_duration_since(Instant::now()),
    )
    .await?;
    if out["lost"] == json!(true) {
        bail!(
            "connection lost after {} of {} bytes",
            out["sent"],
            out["size"]
        );
    }
    out["ev"] = json!("fs_ack");
    Ok(out)
}

/// Read a file of the server's machine (FsRead), in parts of `part` bytes
/// (0: in one), into the local file `to` or into the answer (`text`, the
/// first 64 KiB). `background` as for fs_write.
async fn fs_read(session: &mut Session, c: &Value) -> Result<Value> {
    let to = match opt_string(c, "to") {
        Some(to) => Some(std::fs::File::create(&to).with_context(|| format!("create {to}"))?),
        None => None,
    };
    let spec = DownloadSpec {
        path: string(c, "path")?,
        to,
        offset: number(c, "offset", 0),
        length: number(c, "length", 0),
        part: number(c, "part", 0),
    };
    if !session.connected() {
        bail!("not connected");
    }
    session.next_transfer += 1;
    let id = session.next_transfer;
    let background = flag(c, "background");
    session.downloads.push(Download::new(id, spec, background));
    if background {
        return Ok(json!({"ev": "fs_started", "transfer": id}));
    }
    let mut out = wait_transfer(session, id, millis(c, "timeout_ms", 30_000)).await?;
    if out["lost"] == json!(true) {
        bail!("connection lost after {} bytes", out["size"]);
    }
    out["ev"] = json!("fs_data");
    Ok(out)
}

/// The outcome of transfer `id` once it is over.
async fn wait_transfer(session: &mut Session, id: u64, timeout: Duration) -> Result<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(out) = session.finished.remove(&id) {
            return Ok(out);
        }
        let going = session.uploads.iter().any(|u| u.id == id)
            || session.downloads.iter().any(|d| d.id == id);
        if !going {
            bail!("no transfer {id}");
        }
        if Instant::now() >= deadline {
            bail!("transfer {id} not done within {} ms", timeout.as_millis());
        }
        session.pump(deadline).await;
    }
}

/// Upload content to the server's clipboard (ClipboardPut in chunks) and
/// wait for the ClipboardAck. `total_size` may lie about the size.
async fn clipboard_put(session: &mut Session, c: &Value) -> Result<Value> {
    static TRANSFER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let data = content(c)?;
    let mime = opt_string(c, "mime").unwrap_or_else(|| "text/plain".into());
    let chunk = (number(c, "chunk", 64 * 1024) as usize).max(1);
    let total = number(c, "total_size", data.len() as u64);
    let transfer_id = TRANSFER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let started = Instant::now();
    let pieces: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(chunk).collect()
    };
    let count = pieces.len();
    for (n, piece) in pieces.into_iter().enumerate() {
        let first = n == 0;
        session
            .send(Msg::ClipboardPut(ClipboardPut {
                transfer_id,
                mime: if first { mime.clone() } else { String::new() },
                total_size: if first { total } else { 0 },
                data: Bytes::copy_from_slice(piece),
                last: n + 1 == count,
            }))
            .await?;
    }
    let (ok, error) = wait_msg(session, millis(c, "timeout_ms", 30_000), |msg| match msg {
        server_message::Msg::ClipboardAck(ack) if ack.transfer_id == transfer_id => {
            Some(Ok((ack.ok, ack.error.clone())))
        }
        _ => None,
    })
    .await?;
    Ok(json!({
        "ev": "clipboard_ack", "ok": ok, "error": error, "size": data.len(),
        "sha256": transfer::sha256(&data), "ms": ms(started.elapsed()),
    }))
}

/// Run a program on the server's machine by Exec (draft, see exec.rs):
/// wait for its end, or with `background` answer once it started.
async fn exec_start(session: &mut Session, c: &Value) -> Result<Value> {
    if !session.features.iter().any(|f| f == exec::FEATURE) {
        bail!(
            "the server does not offer {:?} (HelloAck.features {:?}): no Exec in the protocol",
            exec::FEATURE,
            session.features
        );
    }
    let strings = |key: &str| -> Vec<String> {
        c.get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let request_id = session.request_id();
    let stdin = c.get("stdin").is_some();
    session
        .send_draft(exec::Client::Start(exec::ExecStart {
            request_id,
            argv: strings("argv"),
            cwd: opt_string(c, "cwd").unwrap_or_default(),
            env: strings("env"),
            stdin,
            timeout_ms: number(c, "limit_ms", 0) as u32,
        }))
        .await?;
    let deadline = Instant::now() + millis(c, "start_timeout_ms", 10_000);
    let exec_id = loop {
        if Instant::now() >= deadline {
            bail!("no ExecStarted within the timeout");
        }
        if session.down() {
            bail!("not connected");
        }
        if let Some(id) = session.exec_started.remove(&request_id) {
            break id?;
        }
        session.pump(deadline).await;
    };
    let background = flag(c, "background");
    session
        .execs
        .insert(exec_id, exec::Run::new(exec_id, background));
    if let Some(text) = opt_string(c, "stdin") {
        session
            .send_draft(exec::Client::Input(exec::ExecInput {
                exec_id,
                data: Bytes::from(text.into_bytes()),
                eof: true,
            }))
            .await?;
    }
    if background {
        return Ok(json!({"ev": "exec_started", "exec": exec_id}));
    }
    let mut out = wait_exec(session, exec_id, millis(c, "timeout_ms", 30_000)).await?;
    out["ev"] = json!("exec_exit");
    Ok(out)
}

/// The end of Exec run `id`.
async fn wait_exec(session: &mut Session, id: u64, timeout: Duration) -> Result<Value> {
    let deadline = Instant::now() + timeout;
    loop {
        match session.execs.get(&id) {
            None => bail!("no exec {id}"),
            Some(run) => {
                if let Some(exit) = &run.exit {
                    return Ok(exit.clone());
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("exec {id} did not end within {} ms", timeout.as_millis());
        }
        if session.down() {
            bail!("not connected");
        }
        session.pump(deadline).await;
    }
}

fn entry_json(entry: &tilr_proto::FsEntry) -> Value {
    let kind = |k: i32| match tilr_proto::FileKind::try_from(k) {
        Ok(tilr_proto::FileKind::File) => "file",
        Ok(tilr_proto::FileKind::Dir) => "dir",
        Ok(tilr_proto::FileKind::Link) => "link",
        _ => "other",
    };
    json!({
        "name": entry.name, "kind": kind(entry.kind), "target": kind(entry.target),
        "size": entry.size, "mtime_ms": entry.mtime_ms, "mode": entry.mode,
    })
}

/// The bytes a command carries: `text`, the content of the local file
/// `from`, or `size` bytes of `fill` repeated (default "0123456789abcdef").
fn content(c: &Value) -> Result<Vec<u8>> {
    if let Some(text) = opt_string(c, "text") {
        return Ok(text.into_bytes());
    }
    if let Some(from) = opt_string(c, "from") {
        return std::fs::read(&from).with_context(|| format!("read {from}"));
    }
    let size = number(c, "size", 0) as usize;
    let fill = opt_string(c, "fill").unwrap_or_else(|| "0123456789abcdef".into());
    if fill.is_empty() {
        bail!("empty \"fill\"");
    }
    Ok(fill.bytes().cycle().take(size).collect())
}

fn flag(c: &Value, key: &str) -> bool {
    flag_or(c, key, false)
}

fn flag_or(c: &Value, key: &str, default: bool) -> bool {
    c.get(key).and_then(Value::as_bool).unwrap_or(default)
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

/// This driver's `Hello.client_id`: made up when it starts.
fn client_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
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
    let viewed_by: Vec<Value> = pane
        .viewed_by
        .iter()
        .map(|v| json!({"client_id": v.client_id, "name": v.client_name, "addr": v.address}))
        .collect();
    map.insert("viewed_by".into(), json!(viewed_by));
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
