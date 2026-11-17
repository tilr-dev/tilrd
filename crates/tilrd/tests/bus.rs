//! Use cases: the bus (`tilr.bus.v1`) beside `tilr.v1`: Hello, the broker's
//! `from`, routing, order, streams and events. Participants are the test runtime
//! and `tilr_bus` clients as the owner on the Unix socket, told apart by their
//! client names, and raw clients that write envelopes by hand.
//!
//! Each change appends its tests as one block at the end of this file.

mod common;

use bytes::Bytes;
use common::bus::{BusOptions, BusServer, TcpAuth, text};
use common::{Client, TIMEOUT};
use futures_util::FutureExt;
use tilr_bus::bus::{
    Call, Cancel, Chunk, End, Envelope, Hello, Lane, MethodSpec, PingReply, PingRequest,
    Reply as WireReply, Select, envelope::Msg, reply,
};
use tilr_bus::{BusError, CallOptions, ConnectOptions, Credential, Handler, Request, codes};

// ---------------------------------------------------------------------------
// bus-broker-core
// ---------------------------------------------------------------------------

fn start() -> BusServer {
    BusServer::start(BusOptions::default())
}

fn hello(version: u32) -> Msg {
    Msg::Hello(Hello {
        protocol_version: version,
        client_name: "raw".into(),
        ..Hello::default()
    })
}

fn call(id: u64, method: &str, payload: &str) -> Msg {
    Msg::Call(Call {
        id,
        method: method.into(),
        payload: Bytes::copy_from_slice(payload.as_bytes()),
        ..Call::default()
    })
}

/// Ping the broker from `raw` and return what came before the answer: the broker
/// handles what one connection sends in order and queues what it sends to one in
/// order, so everything for `raw` from before the ping is in this list.
async fn before_a_ping(raw: &mut common::bus::Raw, id: u64) -> Vec<Msg> {
    raw.send(Msg::Call(Call {
        id,
        method: "bus.ping".into(),
        target: "tilrd".into(),
        ..Call::default()
    }))
    .await;
    let mut before = Vec::new();
    loop {
        match raw.recv().await {
            Some(Msg::Reply(reply)) if reply.id == id => return before,
            Some(msg) => before.push(msg),
            None => panic!("closed while pinging; before it: {before:?}"),
        }
    }
}

fn connection_error(msg: Option<Msg>) -> (String, String) {
    match msg {
        Some(Msg::ConnectionError(error)) => (error.code, error.message),
        other => panic!("expected ConnectionError, got {other:?}"),
    }
}

async fn owner_calls(bus: &tilr_bus::Bus, method: &str, payload: &str) -> Result<String, BusError> {
    bus.call(method, payload.to_owned(), CallOptions::default())
        .await
        .map(|reply| text(&reply.payload))
}

#[tokio::test]
async fn hello_comes_first_and_names_a_version() {
    let server = start();

    // Anything else first: expected_hello, and the connection closes.
    let mut raw = server.raw().await;
    raw.send(call(1, "pane.list", "")).await;
    let (code, _) = connection_error(raw.recv().await);
    assert_eq!(code, codes::EXPECTED_HELLO);
    assert!(raw.closes().await);
    // A message that is not an envelope first: the same.
    let mut raw = server.raw().await;
    raw.send_raw(vec![9, 1, 2]).await;
    let (code, _) = connection_error(raw.recv().await);
    assert_eq!(code, codes::EXPECTED_HELLO);
    assert!(raw.closes().await);

    // A newer version: unsupported_version, and it closes.
    let mut raw = server.raw().await;
    raw.send(hello(2)).await;
    let (code, message) = connection_error(raw.recv().await);
    assert_eq!(code, codes::UNSUPPORTED_VERSION);
    assert_eq!(
        message,
        "unsupported protocol version 2: this server speaks 1"
    );
    assert!(raw.closes().await);

    // On TCP the owner shows the access token; a wrong one, or a credential
    // (no tokens in this build), is denied.
    for auth in [
        TcpAuth::AccessToken("not-the-token".into()),
        TcpAuth::Credential(Credential::from_bytes(Bytes::from_static(b"token"))),
    ] {
        let error = server.tcp(auth).await.unwrap_err();
        assert_eq!(error.code, codes::DENIED, "{error}");
    }
    let owner = server
        .tcp(TcpAuth::AccessToken(server.token()))
        .await
        .expect("the owner on TCP");
    assert_eq!(owner.identity(), "owner");

    // The owner on the Unix socket needs no token; the version is 1.
    let bus = server.owner().await;
    assert_eq!(bus.identity(), "owner");
    assert_eq!(bus.hello_ack().protocol_version, 1);
    assert!(bus.connection_id() > 0);
    assert_eq!(bus.hello_ack().max_frame, tilr_bus::limits::MAX_FRAME);
    assert_eq!(
        bus.hello_ack().server_id,
        std::fs::read_to_string(server.dir.join("state/server-id"))
            .unwrap()
            .trim()
    );

    // A handshake without the subprotocol is tilr.v1, served as before.
    let mut v1 = Client::connect(&server).await;
    assert!(v1.list_panes().await.is_empty());
}

#[tokio::test]
async fn the_first_thing_a_participant_hears_is_its_hello_ack() {
    let server = start();
    let mut raw = server.raw().await;
    let subscriber = server.owner().await;
    let mut events = subscriber.subscribe("test.*").await.unwrap();

    // Meanwhile a pane is created (tilr.v1) and events are emitted.
    let mut v1 = Client::connect(&server).await;
    v1.open_pane().await;
    subscriber.emit("test.a", "before").await.unwrap();
    assert_eq!(events.next().await.unwrap().subject, "test.a");

    raw.send(hello(1)).await;
    let Some(Msg::HelloAck(ack)) = raw.recv().await else {
        panic!("the first envelope is not HelloAck");
    };
    assert_eq!(ack.identity, "owner");
    // Nothing of what happened before Hello follows it.
    let before = before_a_ping(&mut raw, 1).await;
    assert!(before.is_empty(), "after HelloAck: {before:?}");
}

#[tokio::test]
async fn the_broker_answers_bus_ping() {
    let server = start();
    let bus = server.owner().await;
    let pong: PingReply = bus
        .call_typed(
            "bus.ping",
            &PingRequest { nonce: 42 },
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(pong.nonce, 42);
    let json = bus
        .call(
            "bus.ping",
            r#"{"nonce":"7"}"#,
            CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
        )
        .await
        .unwrap();
    assert_eq!(json.content_type, tilr_bus::CONTENT_TYPE_JSON);
    assert_eq!(text(&json.payload), r#"{"nonce":"7"}"#);
}

#[tokio::test]
async fn unknown_envelopes_are_ignored_and_bad_ones_reported() {
    let server = start();
    let mut raw = server.raw_greeted("raw", Vec::new()).await;

    // An unknown flag byte.
    raw.send_raw(vec![9, 1, 2]).await;
    let (code, _) = connection_error(raw.recv().await);
    assert_eq!(code, codes::BAD_FRAME);
    // A plain message that is not an envelope.
    raw.send_raw(vec![0, 0xff, 0xff, 0xff]).await;
    let (code, _) = connection_error(raw.recv().await);
    assert_eq!(code, codes::BAD_FRAME);
    // An envelope of a variant this server does not know (field 99), and a
    // known one with an unknown field (Call field 99): ignored, answered as known.
    raw.send_raw(vec![0, 0x9a, 0x06, 0x00]).await;
    let mut known = codec_plain(Msg::Call(Call {
        id: 5,
        method: "pane.list".into(),
        ..Call::default()
    }));
    // Envelope.call is field 3, length-delimited: append field 99 to its body.
    known[2] += 3;
    known.extend_from_slice(&[0x98, 0x06, 0x01]);
    raw.send_raw(known).await;

    // The connection stays: the call is answered (no pane.* method yet).
    let reply = raw
        .wait_for(|msg| match msg {
            Msg::Reply(reply) => Some(reply.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reply.id, 5);
    let Some(reply::Result::Error(error)) = reply.result else {
        panic!("pane.list is not provided yet: {reply:?}");
    };
    assert_eq!(error.code, codes::NOT_FOUND);
}

/// An envelope framed plain (flag 0), whatever its size.
fn codec_plain(msg: Msg) -> Vec<u8> {
    use prost::Message;
    let mut out = vec![0];
    out.extend_from_slice(&Envelope { msg: Some(msg) }.encode_to_vec());
    out
}

#[tokio::test]
async fn from_is_always_the_callers_identity() {
    let server = start();
    let _t2 = server.test_runtime("t-2", &[]).await;
    let mut t1 = server.raw_greeted("t-1", Vec::new()).await;
    // A forged from.
    t1.send(Msg::Call(Call {
        id: 1,
        method: "test.from".into(),
        from: "tilrd".into(),
        ..Call::default()
    }))
    .await;
    let reply = t1
        .wait_for(|msg| match msg {
            Msg::Reply(reply) => Some(reply.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reply.id, 1);
    let Some(reply::Result::Payload(payload)) = reply.result else {
        panic!("{reply:?}");
    };
    assert_eq!(text(&payload), "owner");

    // A reply is taken only from the connection the call went to: a forged one
    // from another connection is dropped. The provider answers once the broker
    // has taken every forged reply.
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let (started_tx, started) = tokio::sync::oneshot::channel::<()>();
    let started_tx = std::sync::Mutex::new(Some(started_tx));
    let slow = server
        .owner_with(ConnectOptions::new("slow").provide(
            MethodSpec::unary("test.slow"),
            Handler::unary({
                let release = release.clone();
                move |_: Request| {
                    if let Some(tx) = started_tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                    let release = release.clone();
                    async move {
                        release.notified().await;
                        Ok("real")
                    }
                }
            }),
        ))
        .await;
    t1.send(call(2, "test.slow", "")).await;
    started.await.expect("test.slow was called");
    let mut forger = server.raw_greeted("forger", Vec::new()).await;
    for id in 0..64 {
        forger
            .send(Msg::Reply(WireReply {
                id,
                result: Some(reply::Result::Payload(Bytes::from_static(b"forged"))),
                content_type: String::new(),
            }))
            .await;
    }
    before_a_ping(&mut forger, 1000).await;
    release.notify_one();
    let reply = t1
        .wait_for(|msg| match msg {
            Msg::Reply(reply) if reply.id == 2 => Some(reply.clone()),
            _ => None,
        })
        .await;
    assert_eq!(
        reply.result,
        Some(reply::Result::Payload(Bytes::from_static(b"real")))
    );
    drop(slow);
}

#[tokio::test]
async fn calls_go_round_robin_over_the_providers() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    let _t2 = server.test_runtime("t-2", &[]).await;
    let owner = server.owner().await;
    let mut answers = Vec::new();
    for _ in 0..4 {
        answers.push(owner_calls(&owner, "test.whoami", "").await.unwrap());
    }
    assert_eq!(answers, ["t-1", "t-2", "t-1", "t-2"]);

    // No provider of a method: unavailable; a target that is not connected too.
    let error = owner_calls(&owner, "nobody.here", "").await.unwrap_err();
    assert_eq!(error.code, codes::UNAVAILABLE);
    assert_eq!(error.message, "no provider of nobody.here is connected");
    let error = owner
        .call(
            "test.whoami",
            Bytes::new(),
            CallOptions::default().target("instance:t-9"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::UNAVAILABLE);
}

/// A window providing `ui.notify` for the last active one; it answers its name.
async fn window(server: &BusServer, name: &'static str) -> tilr_bus::Bus {
    server
        .owner_with(ConnectOptions::new(name).provide(
            MethodSpec::unary("ui.notify").with_select(Select::LastActive),
            Handler::unary(move |_: Request| async move { Ok(name) }),
        ))
        .await
}

/// Activate, and know the broker has taken it: what one connection sends is
/// handled in order, so the ping's reply comes after.
async fn activate(bus: &tilr_bus::Bus) {
    bus.activate().await.unwrap();
    let _: PingReply = bus
        .call_typed(
            "bus.ping",
            &PingRequest { nonce: 1 },
            CallOptions::default(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn last_active_goes_to_the_provider_that_activated_last() {
    let server = start();
    let w1 = window(&server, "w-1").await;
    let w2 = window(&server, "w-2").await;
    let owner = server.owner().await;
    // None activated: the last connected.
    assert_eq!(owner_calls(&owner, "ui.notify", "").await.unwrap(), "w-2");
    activate(&w2).await;
    activate(&w1).await;
    for _ in 0..3 {
        assert_eq!(owner_calls(&owner, "ui.notify", "").await.unwrap(), "w-1");
    }
    // The active one gone (the broker logs it once it has let go): the other.
    let gone = format!("connection={} ", w1.connection_id());
    drop(w1);
    server
        .wait_logged_event("w-1 is not gone", |lines| {
            lines
                .iter()
                .any(|line| line.contains("bus participant gone") && line.contains(&gone))
        })
        .await;
    assert_eq!(owner_calls(&owner, "ui.notify", "").await.unwrap(), "w-2");
    drop(w2);
}

#[tokio::test]
async fn reserved_namespaces_cannot_be_provided() {
    let server = start();
    for method in [
        "pane.create",
        "bus.ping",
        "server.info",
        "fs.read",
        "exec.run",
        "clipboard.put",
    ] {
        let error = server
            .try_owner_with(ConnectOptions::new("pretender").provide(
                MethodSpec::unary(method),
                Handler::unary(|_: Request| async { Ok("no") }),
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code, codes::RESERVED, "{method}: {error}");
    }
    // Neither can events be emitted there by anyone but tilrd: dropped.
    let owner = server.owner().await;
    let mut events = owner.subscribe("pane.*").await.unwrap();
    owner.emit("pane.created", "forged").await.unwrap();
    let mut probe = owner.subscribe("probe.*").await.unwrap();
    owner.emit("probe.x", "").await.unwrap();
    // The client hands events to its subscriptions in the order they came, so a
    // forged pane.created would be waiting by the time probe.x is.
    assert_eq!(probe.next().await.unwrap().subject, "probe.x");
    assert!(events.next().now_or_never().is_none());
    // And an instance of bus.toml is refused the same way.
    a_reserved_method_from_an_instance().await;
}

#[tokio::test]
async fn calls_between_two_participants_keep_their_order() {
    let server = start();
    let mut t2 = server
        .raw_greeted("t-2", vec![MethodSpec::unary("test.record")])
        .await;
    let mut t1 = server.raw_greeted("t-1", Vec::new()).await;
    const CALLS: u64 = 1000;
    for i in 0..CALLS {
        t1.send(call(i + 1, "test.record", &i.to_string())).await;
    }
    let mut seen = Vec::new();
    while seen.len() < CALLS as usize {
        match t2.recv().await {
            Some(Msg::Call(call)) => {
                assert_eq!(call.from, "owner");
                seen.push(text(&call.payload));
                t2.send(Msg::Reply(WireReply {
                    id: call.id,
                    result: Some(reply::Result::Payload(call.payload)),
                    content_type: String::new(),
                }))
                .await;
            }
            other => panic!("expected a call, got {other:?}"),
        }
    }
    let expected: Vec<String> = (0..CALLS).map(|i| i.to_string()).collect();
    assert_eq!(seen, expected);
    let mut replies = 0;
    while replies < CALLS {
        if let Some(Msg::Reply(reply)) = t1.recv().await {
            replies += 1;
            assert!(matches!(reply.result, Some(reply::Result::Payload(_))));
        }
    }
}

#[tokio::test]
async fn a_stream_is_a_call_answered_in_chunks() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    let owner = server.owner().await;

    // Count to three.
    let mut stream = owner
        .stream("test.count", "3", tilr_bus::StreamOptions::default())
        .await
        .unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(text(&chunk.unwrap().payload));
    }
    assert_eq!(chunks, ["1", "2", "3"]);
    assert!(stream.end_payload().is_some(), "the stream ended with End");

    // Cancelled after the first chunk: the provider hears of it, no End comes.
    let mut raw = server.raw_greeted("raw", Vec::new()).await;
    // A small window it never credits: the provider waits, the caller's lane
    // stays short (credits are relayed, not enforced, before bus-streams-events).
    raw.send(Msg::Call(Call {
        id: 7,
        method: "test.count".into(),
        payload: Bytes::from_static(b"1000000"),
        window: 16,
        ..Call::default()
    }))
    .await;
    let first = raw
        .wait_for(|msg| match msg {
            Msg::Chunk(chunk) => Some(chunk.clone()),
            other => panic!("expected a chunk, got {other:?}"),
        })
        .await;
    assert_eq!((first.id, text(&first.payload).as_str()), (7, "1"));
    raw.send(Msg::Cancel(Cancel {
        id: 7,
        reason: "enough".into(),
    }))
    .await;
    // `test.records 1` answers once the provider recorded something: only a
    // cancelled handler records here. Its reply comes after the chunks the
    // provider sent, so those are queued for `raw` before the ping's answer.
    let records = owner_calls(&owner, "test.records", "1").await.unwrap();
    assert!(
        records
            .lines()
            .any(|line| line.starts_with("cancelled test.count")),
        "no cancel recorded: {records}"
    );
    // Chunks already on their way may still come; no End, no reply.
    for msg in before_a_ping(&mut raw, 8).await {
        match msg {
            Msg::Chunk(chunk) => assert_eq!(chunk.id, 7),
            other => panic!("after Cancel: {other:?}"),
        }
    }

    // A provider answering a stream with a reply: the caller gets bad_reply.
    let mut provider = server
        .raw_greeted(
            "bad-stream",
            vec![MethodSpec::stream("test.badstream", Lane::Interactive)],
        )
        .await;
    let mut caller = server.raw_greeted("caller", Vec::new()).await;
    caller.send(call(3, "test.badstream", "")).await;
    let hop = provider
        .wait_for(|msg| match msg {
            Msg::Call(call) => {
                assert_eq!(call.method, "test.badstream");
                Some(call.id)
            }
            _ => None,
        })
        .await;
    provider
        .send(Msg::Chunk(Chunk {
            id: hop,
            payload: Bytes::from_static(b"one"),
            content_encoding: String::new(),
        }))
        .await;
    provider
        .send(Msg::Reply(WireReply {
            id: hop,
            result: Some(reply::Result::Payload(Bytes::from_static(b"oops"))),
            content_type: String::new(),
        }))
        .await;
    let Some(Msg::Chunk(chunk)) = caller.recv().await else {
        panic!("the chunk first");
    };
    assert_eq!(chunk.id, 3);
    let Some(Msg::Reply(reply)) = caller.recv().await else {
        panic!("then the reply");
    };
    let Some(reply::Result::Error(error)) = reply.result else {
        panic!("{reply:?}");
    };
    assert_eq!(error.code, codes::BAD_REPLY);
    // And End on a stream after it is dropped: nothing more for the caller.
    provider
        .send(Msg::End(End {
            id: hop,
            payload: Bytes::new(),
        }))
        .await;
    before_a_ping(&mut provider, 100).await;
    let before = before_a_ping(&mut caller, 101).await;
    assert!(before.is_empty(), "after bad_reply: {before:?}");
}

#[tokio::test]
async fn events_reach_the_subscribers_connected_now() {
    let server = start();
    let t1 = server.owner().await;
    let t2 = server.owner().await;
    let t3 = server.owner().await;
    let mut star = t2.subscribe("test.*").await.unwrap();
    let mut tail = t3.subscribe("test.>").await.unwrap();

    t1.emit("test.a", "x").await.unwrap();
    t1.emit("test.a.b", "y").await.unwrap();
    t1.emit("test.end", "").await.unwrap();

    let event = star.next().await.unwrap();
    assert_eq!(
        (event.subject.as_str(), event.from.as_str()),
        ("test.a", "owner")
    );
    assert_eq!(text(&event.payload), "x");
    // test.a.b is not one segment: the next is the end marker.
    assert_eq!(star.next().await.unwrap().subject, "test.end");
    let subjects: Vec<String> = [
        tail.next().await.unwrap(),
        tail.next().await.unwrap(),
        tail.next().await.unwrap(),
    ]
    .into_iter()
    .map(|event| event.subject)
    .collect();
    assert_eq!(subjects, ["test.a", "test.a.b", "test.end"]);

    // No history: a later subscriber gets only later events.
    t1.emit("test.a", "old").await.unwrap();
    let t4 = server.owner().await;
    let mut late = t4.subscribe("test.*").await.unwrap();
    t1.emit("test.b", "new").await.unwrap();
    let event = late.next().await.unwrap();
    assert_eq!(
        (event.subject.as_str(), text(&event.payload).as_str()),
        ("test.b", "new")
    );

    // A subscriber with two matching patterns gets an event once.
    let mut also = t4.subscribe("test.>").await.unwrap();
    t1.emit("test.c", "").await.unwrap();
    t1.emit("test.end", "").await.unwrap();
    assert_eq!(late.next().await.unwrap().subject, "test.c");
    assert_eq!(late.next().await.unwrap().subject, "test.end");
    assert_eq!(also.next().await.unwrap().subject, "test.c");
}

#[tokio::test]
async fn a_participant_with_the_same_client_id_supersedes_the_older() {
    let server = start();
    let first = server
        .owner_with(ConnectOptions::new("app").client_id("window-1"))
        .await;
    let second = server
        .owner_with(ConnectOptions::new("app").client_id("window-1"))
        .await;
    let closed = tokio::time::timeout(TIMEOUT, first.closed()).await.unwrap();
    assert_eq!(closed.code, Some(tilr_bus::close::SUPERSEDED));
    assert!(!second.is_closed());
}

#[tokio::test]
async fn the_end_of_a_relayed_bulk_stream_comes_after_its_chunks() {
    let server = start();
    let mut provider = server
        .raw_greeted("bulk", vec![MethodSpec::stream("test.bulk", Lane::Bulk)])
        .await;
    let mut caller = server.raw_greeted("caller", Vec::new()).await;
    const CHUNKS: usize = 100;
    // Ended by End, then by an error reply.
    for (id, error) in [(1, false), (2, true)] {
        caller.send(call(id, "test.bulk", "")).await;
        let hop = provider
            .wait_for(|msg| match msg {
                Msg::Call(call) => Some(call.id),
                _ => None,
            })
            .await;
        for i in 0..CHUNKS {
            provider
                .send(Msg::Chunk(Chunk {
                    id: hop,
                    payload: Bytes::from(vec![i as u8; 16 << 10]),
                    content_encoding: String::new(),
                }))
                .await;
        }
        let last = if error {
            Msg::Reply(WireReply {
                id: hop,
                result: Some(reply::Result::Error(tilr_bus::bus::Error {
                    code: "test_failed".into(),
                    message: "failed after its chunks".into(),
                })),
                content_type: String::new(),
            })
        } else {
            Msg::End(End {
                id: hop,
                payload: Bytes::from_static(b"done"),
            })
        };
        provider.send(last).await;
        let mut chunks = 0;
        loop {
            match caller.recv().await {
                Some(Msg::Chunk(chunk)) => {
                    assert_eq!(chunk.id, id);
                    chunks += 1;
                }
                Some(Msg::End(end)) => {
                    assert!(!error);
                    assert_eq!((end.id, &end.payload[..]), (id, &b"done"[..]));
                    break;
                }
                Some(Msg::Reply(reply)) => {
                    assert!(error, "{reply:?}");
                    assert_eq!(reply.id, id);
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(chunks, CHUNKS, "the end came before the last chunk");
    }
}

// ---------------------------------------------------------------------------
// bus-instances
// ---------------------------------------------------------------------------

use common::bus_instances::{
    WRITTEN, alive, bounded, info, instance, instance_socket, instance_with, listing, method_list,
    module, reload, reload_config, script_module, start as start_instance, stop as stop_instance,
    test_module, token_module, wait_gone, wait_instance, wait_tokens, wait_up, write_bus_toml,
};
use std::time::Duration;

use tilr_bus::bus::reload_request::What;

/// A server with `bus.toml` and `env` more in its environment.
fn instances_server(bus_toml: String, env: &[(&str, &str)]) -> BusServer {
    BusServer::start(BusOptions {
        bus_toml: Some(bus_toml),
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ..BusOptions::default()
    })
}

/// A call to one instance by its identity.
fn to(instance: &str) -> CallOptions {
    CallOptions::default().target(format!("instance:{instance}"))
}

/// [`owner_calls`] within [`TIMEOUT`].
async fn calls(bus: &tilr_bus::Bus, method: &str, payload: &str) -> Result<String, BusError> {
    bounded(method, owner_calls(bus, method, payload)).await
}

#[tokio::test]
async fn an_instance_from_bus_toml_is_started_and_answers() {
    let server = instances_server(
        format!(
            "{}{}{}",
            test_module("t", &[]),
            instance("t-1", "t"),
            instance_with(
                "t-2",
                "t",
                "autostart = false\n",
                &[("provide", &["test.*"])],
                &[]
            )
        ),
        &[],
    );
    let owner = server.owner().await;

    let t1 = wait_up(&server, &owner, "t-1").await;
    assert!(t1.pid > 0 && alive(t1.pid), "{t1:?}");
    assert_eq!((t1.module.as_str(), t1.transport.as_str()), ("t", "fd"));
    assert!(t1.provides.contains(&"test.echo".to_owned()), "{t1:?}");
    assert_eq!(calls(&owner, "test.echo", "hi").await.unwrap(), "hi");
    // Its identity is `instance:<name>`, set by the broker.
    assert_eq!(
        calls(&owner, "test.whoami", "").await.unwrap(),
        "instance:t-1"
    );

    // `autostart = false`: listed as stopped, no process until bus.start.
    let t2 = info(&owner, "t-2").await;
    assert_eq!((t2.state.as_str(), t2.pid), ("stopped", 0), "{t2:?}");
    let started = start_instance(&owner, "t-2").await.unwrap();
    let t2 = wait_up(&server, &owner, "t-2").await;
    assert_eq!(started.pid, t2.pid);
    assert!(alive(t2.pid));

    let error = start_instance(&owner, "t-9").await.unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND, "{error}");
}

#[tokio::test]
async fn a_bad_bus_toml_leaves_panes_working() {
    // A misspelt key: logged with the file and the key, no instances, panes as usual.
    let server = instances_server(
        format!("{}[instances.t-1]\nmodul = \"t\"\n", test_module("t", &[])),
        &[],
    );
    server.wait_logged("modul").await;
    let line = server
        .log_lines()
        .into_iter()
        .find(|line| line.contains("modul"))
        .unwrap();
    assert!(line.contains("bus.toml"), "{line}");
    let mut v1 = Client::connect(&server).await;
    v1.open_pane().await;
    let owner = server.owner().await;
    assert!(listing(&owner).await.instances.is_empty());

    // The first round's key names its replacement.
    let server = instances_server(
        format!(
            "{}[instances.t-1]\nmodule = \"t\"\ncalls = [\"test.*\"]\n",
            test_module("t", &[])
        ),
        &[],
    );
    server.wait_logged("`calls` was replaced by").await;
    let line = server
        .log_lines()
        .into_iter()
        .find(|line| line.contains("`calls` was replaced by"))
        .unwrap();
    assert!(line.contains("[instances.t-1.rights] call"), "{line}");
    let owner = server.owner().await;
    assert!(listing(&owner).await.instances.is_empty());

    // After the file is mended, reload_config starts what it names.
    write_bus_toml(
        &server,
        &format!("{}{}", test_module("t", &[]), instance("t-1", "t")),
    );
    let done = reload_config(&owner).await.unwrap();
    assert_eq!(done.started, ["t-1"]);
    wait_up(&server, &owner, "t-1").await;
}

#[tokio::test]
async fn reloading_a_bad_bus_toml_changes_nothing() {
    let good = format!("{}{}", test_module("t", &[]), instance("t-1", "t"));
    let server = instances_server(good.clone(), &[]);
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "t-1").await;

    // Misspelt key, wrong type, the old key, an instance without its module: every
    // one is `bad_config` and the instance runs on with the same pid.
    for bad in [
        format!("{good}[instances.t-2]\nmodul = \"t\"\n"),
        format!("{good}[instances.t-2]\nmodule = \"t\"\nautostart = 3\n"),
        format!("{good}[instances.t-2]\nmodule = \"t\"\ncalls = []\n"),
        format!("{good}[instances.t-2]\nmodule = \"nope\"\n"),
        format!("{good}[instances.owner]\nmodule = \"t\"\n"),
    ] {
        write_bus_toml(&server, &bad);
        let error = reload_config(&owner).await.unwrap_err();
        assert_eq!(error.code, codes::BAD_CONFIG, "{error}");
        assert!(error.message.contains("bus.toml"), "{error}");
        let after = info(&owner, "t-1").await;
        assert_eq!((after.pid, after.state.as_str()), (before.pid, "up"));
        assert_eq!(listing(&owner).await.instances.len(), 1);
    }
    let error = {
        write_bus_toml(&server, &format!("{good}[instances.t-2]\ncalls = 1\n"));
        reload_config(&owner).await.unwrap_err()
    };
    assert_eq!(error.code, codes::BAD_CONFIG);
    assert_eq!(info(&owner, "t-1").await.pid, before.pid);

    // The same file again changes nothing either.
    write_bus_toml(&server, &good);
    let done = reload_config(&owner).await.unwrap();
    assert_eq!(done.kept, ["t-1"]);
    assert!(done.started.is_empty() && done.stopped.is_empty() && done.reloaded.is_empty());
    assert_eq!(info(&owner, "t-1").await.pid, before.pid);
}

#[tokio::test]
async fn a_socket_path_that_is_too_long_is_a_config_error() {
    let name = "x".repeat(120);
    let server = instances_server(
        format!(
            "{}{}",
            test_module("t", &[]),
            instance_with(&name, "t", "connect = \"socket\"\n", &[], &[])
        ),
        &[],
    );
    server.wait_logged("socket path").await;
    let line = server
        .log_lines()
        .into_iter()
        .find(|line| line.contains("socket path"))
        .unwrap();
    assert!(line.contains("bus.toml"), "{line}");
    let owner = server.owner().await;
    assert!(listing(&owner).await.instances.is_empty());
    // The same name over descriptor 3 has no socket and is fine.
    write_bus_toml(
        &server,
        &format!(
            "{}{}",
            test_module("t", &[]),
            instance_with(&name, "t", "", &[("provide", &["test.*"])], &[])
        ),
    );
    reload_config(&owner).await.unwrap();
    wait_up(&server, &owner, &name).await;
}

#[tokio::test]
async fn an_instance_gets_its_connection_and_names() {
    let server = instances_server(
        format!(
            "{}{}",
            test_module("t", &[("X", "module"), ("ONLY_MODULE", "m")]),
            instance_with(
                "t-1",
                "t",
                "",
                &[("provide", &["test.*"])],
                &[("X", "instance")]
            )
        ),
        &[
            ("TILR_PANE", "pane-7"),
            ("TILR_SOCK", "/nowhere"),
            ("TILR_BUS_SOCK", "/nowhere-either"),
            ("KEPT_FROM_SERVER", "yes"),
        ],
    );
    let owner = server.owner().await;
    wait_up(&server, &owner, "t-1").await;
    let report = calls(&owner, "test.env", "").await.unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    let env = &report["env"];
    assert_eq!(env["TILR_BUS_FD"], "3");
    assert_eq!(env["TILR_INSTANCE"], "t-1");
    assert_eq!(
        env["TILR_CONFIG_DIR"],
        server.dir.join("config").to_str().unwrap()
    );
    // The instance's env over the module's; the module's own where the instance says none.
    assert_eq!(env["X"], "instance");
    assert_eq!(env["ONLY_MODULE"], "m");
    // The server's environment, without the pane's and the bus's variables.
    assert_eq!(env["KEPT_FROM_SERVER"], "yes");
    for gone in ["TILR_PANE", "TILR_SOCK", "TILR_BUS_SOCK", "TILR_BUS_TOKEN"] {
        assert!(env.get(gone).is_none(), "{gone} reached the instance");
    }
    // Standard input, output, error and its connection: nothing else of the server.
    assert_eq!(report["fds"], serde_json::json!([0, 1, 2, 3]));
}

#[tokio::test]
async fn a_socket_instance_connects_once_with_its_token() {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncReadExt;
    // A connection that says nothing is closed after the Hello limit (10 s; a test
    // build shortens it to 2 s).
    let server = instances_server(
        format!(
            "{}{}",
            test_module("t", &[]),
            instance_with(
                "t-1",
                "t",
                "connect = \"socket\"\n",
                &[("provide", &["test.*"])],
                &[]
            )
        ),
        &[("TILR_TEST_BUS_SOCKET_HELLO_MS", "2000")],
    );
    let owner = server.owner().await;
    let up = wait_up(&server, &owner, "t-1").await;
    assert_eq!(up.transport, "socket");
    assert_eq!(calls(&owner, "test.echo", "x").await.unwrap(), "x");

    let report = calls(&owner, "test.env", "").await.unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    let env = &report["env"];
    let socket = instance_socket(&server, "t-1");
    assert_eq!(env["TILR_BUS_SOCK"], socket.to_str().unwrap());
    let token = env["TILR_BUS_TOKEN"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "{token}");
    assert!(env.get("TILR_BUS_FD").is_none());
    // Descriptors 0, 1 and 2 only: the socket is connected to after they are probed.
    assert_eq!(report["fds"], serde_json::json!([0, 1, 2]));
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&socket), 0o600);
    assert_eq!(mode(socket.parent().unwrap()), 0o700);

    // The token was taken once: the same one again, and a wrong one, are refused.
    for token in [token.clone(), "0".repeat(32)] {
        let thief = tilr_bus::Bus::connect(
            tilr_bus::Transport::Socket {
                path: socket.clone(),
                token,
            },
            ConnectOptions::new("thief"),
        )
        .await
        .expect_err("the instance socket takes its token once");
        assert_eq!(thief.code, codes::DENIED, "{thief}");
    }
    let after = info(&owner, "t-1").await;
    assert_eq!((after.pid, after.state.as_str()), (up.pid, "up"));
    assert_eq!(after.restarts, 0);

    // A connection that sends nothing is closed, and the instance does not mind.
    let mut silent = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(common::TIMEOUT, silent.read(&mut byte))
        .await
        .expect("the silent connection is closed");
    assert_eq!(read.unwrap_or(0), 0);
    let after = info(&owner, "t-1").await;
    assert_eq!((after.pid, after.state.as_str()), (up.pid, "up"));
}

#[tokio::test]
async fn a_crashed_instance_is_restarted_with_backoff() {
    let server = instances_server(
        format!(
            "{}{}",
            script_module("c", "exit 0", &[]),
            instance_with("c-1", "c", "", &[], &[])
        ),
        &[],
    );
    let owner = server.owner().await;
    // Pauses of 200, 400 and 800 ms, as the server logs each before it waits it out (the
    // log says what the runner chose; a clock read between polls does not, under load).
    let marker = "instance c-1 starts again in ";
    server.wait_logged(&format!("{marker}800 ms")).await;
    let pauses: Vec<u64> = server
        .log_lines()
        .iter()
        .filter_map(|line| {
            let rest = &line[line.find(marker)? + marker.len()..];
            rest.split_whitespace().next()?.parse().ok()
        })
        .collect();
    assert_eq!(pauses[..3], [200, 400, 800], "{pauses:?}");
    wait_instance(&server, &owner, "c-1", "a restart", |info| {
        info.restarts >= 3
    })
    .await;
    // Between processes it is `down`, with no pid; a stop ends the loop.
    stop_instance(&owner, "c-1", None).await.unwrap();
    let after = info(&owner, "c-1").await;
    assert_eq!((after.state.as_str(), after.pid), ("stopped", 0));
    assert!(after.restarts >= 3, "{after:?}");
}

#[tokio::test]
async fn an_instance_that_loses_its_connection_is_restarted() {
    let server = instances_server(
        format!("{}{}", test_module("t", &[]), instance("t-1", "t")),
        &[],
    );
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "t-1").await;
    assert_eq!(before.restarts, 0);
    // It closes descriptor 3 and keeps running: its identity is the descriptor, it
    // cannot reconnect, so the process is killed and a new one says Hello.
    assert_eq!(
        calls(&owner, "test.close_bus", "").await.unwrap(),
        "closing"
    );
    let after = wait_instance(&server, &owner, "t-1", "a new process", |info| {
        info.state == "up" && info.pid != before.pid
    })
    .await;
    assert_eq!(after.restarts, 1);
    wait_gone(before.pid, "the old process").await;
    assert_eq!(calls(&owner, "test.echo", "again").await.unwrap(), "again");
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_instance_that_never_says_hello_is_restarted() {
    // The limit is 30 s; a test build takes it from the environment.
    let server = instances_server(
        format!(
            "{}{}",
            test_module("t", &[("TEST_NO_HELLO", "1")]),
            instance("t-1", "t")
        ),
        &[("TILR_TEST_BUS_HELLO_MS", "1000")],
    );
    let owner = server.owner().await;
    let first = wait_instance(&server, &owner, "t-1", "a process", |info| info.pid > 0).await;
    assert_eq!(first.state, "starting");
    let second = wait_instance(&server, &owner, "t-1", "a new process", |info| {
        info.restarts >= 1 && info.pid > 0 && info.pid != first.pid
    })
    .await;
    assert_eq!(second.state, "starting");
    assert!(
        !alive(first.pid),
        "the process that said nothing was killed"
    );
}

#[tokio::test]
async fn an_instance_that_cannot_start_is_reported() {
    let server = instances_server(
        format!(
            "{}{}{}{}{}",
            module("gone", &["/nonexistent/program"], &[]),
            script_module("quits", "exit 3", &[]),
            instance_with("gone-1", "gone", "autostart = false\n", &[], &[]),
            instance_with("quits-1", "quits", "autostart = false\n", &[], &[]),
            instance_with("cwd-1", "gone", "autostart = false\n", &[], &[]),
        ),
        &[],
    );
    let owner = server.owner().await;
    let error = start_instance(&owner, "gone-1").await.unwrap_err();
    assert_eq!(error.code, codes::SPAWN_FAILED, "{error}");
    assert!(error.message.contains("/nonexistent/program"), "{error}");
    let error = start_instance(&owner, "quits-1").await.unwrap_err();
    assert_eq!(error.code, codes::START_FAILED, "{error}");
    assert!(error.message.contains("before it said Hello"), "{error}");
    // Both are supervised: they are tried again (restarts count).
    wait_instance(&server, &owner, "quits-1", "a restart", |info| {
        info.restarts >= 1
    })
    .await;
}

#[tokio::test]
async fn an_instance_is_stopped_and_started() {
    let server = instances_server(
        format!("{}{}", test_module("t", &[]), instance("t-1", "t")),
        &[],
    );
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "t-1").await;

    // A call in flight is answered before the process is stopped (drained).
    let mut sleeping = Box::pin(owner.call("test.sleep", "600", to("t-1")));
    // Polled once, so the call is sent.
    let _ = sleeping.as_mut().now_or_never();
    // Calls of one connection reach the broker in order: once this is answered, the
    // sleep has been routed.
    calls(&owner, "test.echo", "sync").await.unwrap();
    let stopping = stop_instance(&owner, "t-1", None);
    let (slept, stopped) = bounded("the drained stop", async {
        tokio::join!(sleeping, stopping)
    })
    .await;
    assert_eq!(text(&slept.unwrap().payload), "slept");
    stopped.unwrap();
    let after = info(&owner, "t-1").await;
    assert_eq!(
        (after.state.as_str(), after.pid),
        ("stopped", 0),
        "{after:?}"
    );
    assert!(!alive(before.pid));
    // No new calls: nobody provides test.echo.
    let error = calls(&owner, "test.echo", "x").await.unwrap_err();
    assert_eq!(error.code, codes::UNAVAILABLE, "{error}");
    // Stopping again is no error. A stop ends the instance's task (logged as `is
    // stopped`): nothing is left that would start it again after a pause.
    stop_instance(&owner, "t-1", None).await.unwrap();
    let still = info(&owner, "t-1").await;
    assert_eq!((still.state.as_str(), still.restarts), ("stopped", 0));
    let lines = server.log_lines();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("instance t-1 is stopped")),
        "{lines:?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("instance t-1 starts again")),
        "{lines:?}"
    );

    // Started, it answers once it said Hello.
    let started = start_instance(&owner, "t-1").await.unwrap();
    assert!(started.pid > 0 && started.pid != before.pid);
    assert_eq!(info(&owner, "t-1").await.state, "up");
    assert_eq!(calls(&owner, "test.echo", "back").await.unwrap(), "back");
    // Started while it runs: its pid.
    assert_eq!(
        start_instance(&owner, "t-1").await.unwrap().pid,
        started.pid
    );
}

#[tokio::test]
async fn an_instance_is_stopped_with_sigterm() {
    // It says when SIGTERM came: an instance started with the signal blocked (as the
    // supervisor's threads hold it) would hear nothing and be killed after 3 s.
    let server = instances_server(
        format!(
            "{}{}",
            script_module(
                "term",
                "trap 'echo got TERM >&2; exit 0' TERM; echo ready >&2; sleep 3600 & wait $!",
                &[]
            ),
            instance_with("t-1", "term", "", &[], &[])
        ),
        &[],
    );
    let owner = server.owner().await;
    server.wait_logged("bus t-1: ready").await;
    stop_instance(&owner, "t-1", Some(0)).await.unwrap();
    server.wait_logged("bus t-1: got TERM").await;
    assert_eq!(info(&owner, "t-1").await.state, "stopped");
}

#[tokio::test]
async fn an_instance_that_ignores_sigterm_is_killed_after_3_s() {
    let runtime = common::bus::test_runtime_path();
    let server = instances_server(
        format!(
            "{}{}",
            script_module(
                "stubborn",
                &format!("trap '' TERM; exec {}", runtime.display()),
                &[]
            ),
            instance("s-1", "stubborn")
        ),
        &[],
    );
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "s-1").await;
    let began = std::time::Instant::now();
    stop_instance(&owner, "s-1", Some(0)).await.unwrap();
    assert!(
        began.elapsed() >= Duration::from_secs(3),
        "{:?}",
        began.elapsed()
    );
    assert!(!alive(before.pid));
    assert_eq!(info(&owner, "s-1").await.state, "stopped");
}

#[tokio::test]
async fn an_instances_own_processes_end_with_it() {
    let dir = std::env::temp_dir().join(format!("tilr-bus-group-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (stopped_pids, crashed_pids) = (dir.join("stopped"), dir.join("crashed"));
    let runtime = common::bus::test_runtime_path();
    // Each starts a process of its own in the background and writes its pid; `g-1` then
    // runs the test runtime, `c-1` exits at once (a crash, restarted with backoff).
    let server = instances_server(
        format!(
            "{}{}{}{}",
            script_module(
                "g",
                &format!(
                    "sleep 3600 & echo $! >> \"$PID_FILE\"; {WRITTEN}; exec {}",
                    runtime.display()
                ),
                &[("PID_FILE", stopped_pids.to_str().unwrap())]
            ),
            instance("g-1", "g"),
            script_module(
                "c",
                &format!("sleep 3600 & echo $! >> \"$PID_FILE\"; {WRITTEN}; exit 0"),
                &[("PID_FILE", crashed_pids.to_str().unwrap())]
            ),
            instance_with("c-1", "c", "", &[], &[]),
        ),
        &[],
    );
    let owner = server.owner().await;

    // Stopped: SIGTERM and SIGKILL go to its process group, the background process too.
    wait_up(&server, &owner, "g-1").await;
    let child: u32 = wait_tokens(&server, "g-1", &stopped_pids, 1).await[0]
        .parse()
        .unwrap();
    assert!(alive(child));
    stop_instance(&owner, "g-1", Some(0)).await.unwrap();
    wait_gone(child, "the stopped instance's own process").await;

    // Its leader exited by itself: what the group still holds is killed before the
    // leader is reaped, every time it is started again.
    let pids = wait_tokens(&server, "c-1", &crashed_pids, 2).await;
    for pid in &pids[..2] {
        wait_gone(pid.parse().unwrap(), "the crashed instance's own process").await;
    }
    stop_instance(&owner, "c-1", None).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_reload_answers_a_start_error() {
    let marker = std::env::temp_dir().join(format!("tilr-bus-fail-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let runtime = common::bus::test_runtime_path();
    // It runs the test runtime until the marker file exists, then exits at its start.
    let server = instances_server(
        format!(
            "{}{}",
            script_module(
                "f",
                &format!("[ -e \"$MARKER\" ] && exit 3; exec {}", runtime.display()),
                &[("MARKER", marker.to_str().unwrap())]
            ),
            instance("f-1", "f"),
        ),
        &[],
    );
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "f-1").await;
    std::fs::write(&marker, "").unwrap();
    let error = reload(&owner, What::Instance("f-1".into()))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::START_FAILED, "{error}");
    assert!(error.message.contains("f-1"), "{error}");
    assert!(!alive(before.pid));
    // It stays supervised: once it can start again, it is up.
    let _ = std::fs::remove_file(&marker);
    let after = wait_up(&server, &owner, "f-1").await;
    assert_ne!(after.pid, before.pid);
}

#[tokio::test]
async fn reloading_a_module_takes_the_right_over_every_instance() {
    let tokens = std::env::temp_dir().join(format!("tilr-bus-probe-{}", std::process::id()));
    let _ = std::fs::remove_file(&tokens);
    let server = instances_server(
        format!(
            "{}{}{}{}",
            test_module("t", &[]),
            instance("t-1", "t"),
            token_module("s", &tokens),
            instance_with(
                "s-1",
                "s",
                "connect = \"socket\"\n",
                &[("call", &["bus.reload"]), ("control", &["reload:s-1"])],
                &[]
            ),
        ),
        &[],
    );
    let owner = server.owner().await;
    let t1 = wait_up(&server, &owner, "t-1").await;
    let first = wait_tokens(&server, "s-1", &tokens, 1).await;
    let me = tilr_bus::Bus::connect(
        tilr_bus::Transport::Socket {
            path: instance_socket(&server, "s-1"),
            token: first[0].clone(),
        },
        ConnectOptions::new("s-1-driver"),
    )
    .await
    .expect("the instance's own connection");
    // A module that exists, one that does not, all: each is `denied` alike, so a caller
    // that may reload only itself learns nothing of the modules or what runs.
    for what in [
        What::Module("t".into()),
        What::Module("nope".into()),
        What::All(true),
    ] {
        let error = me
            .call_typed::<_, tilr_bus::bus::ReloadReply>(
                "bus.reload",
                &tilr_bus::bus::ReloadRequest {
                    what: Some(what.clone()),
                },
                CallOptions::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, codes::DENIED, "{what:?}: {error}");
    }
    assert_eq!(info(&owner, "t-1").await.pid, t1.pid, "t-1 was not touched");
    let _ = std::fs::remove_file(&tokens);
}

#[tokio::test]
async fn a_module_is_reloaded_one_instance_at_a_time() {
    let tokens = std::env::temp_dir().join(format!("tilr-bus-self-{}", std::process::id()));
    let _ = std::fs::remove_file(&tokens);
    let server = instances_server(
        format!(
            "{}{}{}{}{}",
            test_module("t", &[]),
            instance("t-1", "t"),
            instance("t-2", "t"),
            token_module("s", &tokens),
            instance_with(
                "s-1",
                "s",
                "connect = \"socket\"\n",
                &[
                    ("call", &["bus.reload", "bus.stop"]),
                    ("control", &["reload:s-1", "stop:s-1"])
                ],
                &[]
            ),
        ),
        &[],
    );
    let owner = server.owner().await;
    let t1 = wait_up(&server, &owner, "t-1").await;
    let t2 = wait_up(&server, &owner, "t-2").await;

    // The owner calls test.echo in a loop on a connection of its own.
    let caller = server.owner().await;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let looping = {
        let stop = stop.clone();
        tokio::spawn(async move {
            let (mut done, mut failed) = (0u32, Vec::new());
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                match calls(&caller, "test.echo", "ping").await {
                    Ok(_) => done += 1,
                    Err(error) => failed.push(error),
                }
                tokio::task::yield_now().await;
            }
            (done, failed)
        })
    };
    let reloaded = reload(&owner, What::Module("t".into())).await.unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let (done, failed) = bounded("the loop of calls", looping).await.unwrap();
    assert!(
        failed.is_empty(),
        "calls failed during the reload: {failed:?}"
    );
    assert!(done > 0);

    // One at a time, in name order, with the old and the new process.
    assert_eq!(reloaded.reloaded.len(), 2);
    assert_eq!(reloaded.reloaded[0].instance, "t-1");
    assert_eq!(reloaded.reloaded[0].old_pid, t1.pid);
    assert_eq!(reloaded.reloaded[1].instance, "t-2");
    assert_eq!(reloaded.reloaded[1].old_pid, t2.pid);
    for done in &reloaded.reloaded {
        assert_ne!(done.new_pid, done.old_pid);
        assert!(alive(done.new_pid) && !alive(done.old_pid), "{done:?}");
        let now = info(&owner, &done.instance).await;
        assert_eq!((now.state.as_str(), now.pid), ("up", done.new_pid));
        assert_eq!(now.restarts, 0, "a reload is not a restart");
    }
    // Not named, not touched.
    let error = reload(&owner, What::Instance("t-9".into()))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND, "{error}");
    let error = reload(&owner, What::Module("nope".into()))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND, "{error}");

    // A caller that reloads itself has its reply before its process is stopped. The
    // instance here is driven by the test: its process only hands out the token.
    let first = wait_tokens(&server, "s-1", &tokens, 1).await;
    let me = tilr_bus::Bus::connect(
        tilr_bus::Transport::Socket {
            path: instance_socket(&server, "s-1"),
            token: first[0].clone(),
        },
        ConnectOptions::new("s-1-driver"),
    )
    .await
    .expect("the instance's own connection");
    assert_eq!(me.identity(), "instance:s-1");
    let old = wait_up(&server, &owner, "s-1").await;
    let reply: tilr_bus::bus::ReloadReply = me
        .call_typed(
            "bus.reload",
            &tilr_bus::bus::ReloadRequest {
                what: Some(What::Instance("s-1".into())),
            },
            CallOptions::default(),
        )
        .await
        .expect("the reply reaches the instance that asked");
    assert_eq!(reply.reloaded.len(), 1);
    assert_eq!(reply.reloaded[0].old_pid, old.pid);
    // Afterwards its process is stopped and another one started (it hands out a new token).
    let tokens_now = wait_tokens(&server, "s-1", &tokens, 2).await;
    assert_ne!(tokens_now[0], tokens_now[1]);
    wait_instance(&server, &owner, "s-1", "a new process", |info| {
        info.pid > 0 && info.pid != old.pid
    })
    .await;
    assert!(!alive(old.pid));
    bounded("the old connection's end", me.closed()).await;
    // It may not do to others what it has no right to.
    let denied = tilr_bus::Bus::connect(
        tilr_bus::Transport::Socket {
            path: instance_socket(&server, "s-1"),
            token: tokens_now[1].clone(),
        },
        ConnectOptions::new("s-1-again"),
    )
    .await
    .expect("the new process's connection");
    let error = denied
        .call_typed::<_, tilr_bus::bus::ReloadReply>(
            "bus.reload",
            &tilr_bus::bus::ReloadRequest {
                what: Some(What::Instance("t-1".into())),
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
    let _ = std::fs::remove_file(&tokens);
}

#[tokio::test]
async fn reloading_bus_toml_applies_the_difference() {
    let before = |a: &str| {
        format!(
            "{}{}{}{}{}",
            test_module("t", &[]),
            instance_with("t-1", "t", "", &[("provide", &["test.*"])], &[("A", a)]),
            instance("t-3", "t"),
            instance("t-4", "t"),
            instance_with(
                "t-5",
                "t",
                "autostart = false\n",
                &[("provide", &["test.*"])],
                &[]
            ),
        )
    };
    let server = instances_server(before("1"), &[]);
    let owner = server.owner().await;
    let t1 = wait_up(&server, &owner, "t-1").await;
    let t3 = wait_up(&server, &owner, "t-3").await;
    let t4 = wait_up(&server, &owner, "t-4").await;

    // t-2 added, t-1's env changed, t-3 removed; t-4 and the stopped t-5 as they were.
    write_bus_toml(
        &server,
        &format!(
            "{}{}{}",
            before("2")
                .replace(&instance("t-3", "t"), "")
                .replace("A = \"1\"", "A = \"2\""),
            instance("t-2", "t"),
            ""
        ),
    );
    let done = reload_config(&owner).await.unwrap();
    assert_eq!(done.started, ["t-2"]);
    assert_eq!(done.stopped, ["t-3"]);
    assert_eq!(done.reloaded, ["t-1"]);
    assert_eq!(done.kept, ["t-4", "t-5"]);

    let t2 = info(&owner, "t-2").await;
    assert_eq!(t2.state, "up");
    let t1_now = info(&owner, "t-1").await;
    assert_eq!((t1_now.state.as_str(), t1_now.restarts), ("up", 0));
    assert_ne!(t1_now.pid, t1.pid);
    let report = owner_calls_to(&owner, "test.env", "t-1").await;
    assert_eq!(report["env"]["A"], "2");
    assert!(
        listing(&owner)
            .await
            .instances
            .iter()
            .all(|info| info.name != "t-3")
    );
    assert!(!alive(t3.pid), "the removed instance stopped");
    // The unchanged one was not touched; the manually stopped one stays stopped.
    let t4_now = info(&owner, "t-4").await;
    assert_eq!((t4_now.pid, t4_now.state.as_str()), (t4.pid, "up"));
    assert_eq!(info(&owner, "t-5").await.state, "stopped");

    // An instance stopped by hand stays stopped through a reload of the file.
    stop_instance(&owner, "t-4", None).await.unwrap();
    let done = reload_config(&owner).await.unwrap();
    assert!(done.started.is_empty() && done.stopped.is_empty() && done.reloaded.is_empty());
    assert_eq!(info(&owner, "t-4").await.state, "stopped");
}

async fn owner_calls_to(owner: &tilr_bus::Bus, method: &str, instance: &str) -> serde_json::Value {
    let reply = bounded(method, owner.call(method, "", to(instance)))
        .await
        .unwrap_or_else(|error| panic!("{method} on {instance}: {error}"));
    serde_json::from_str(&text(&reply.payload)).unwrap()
}

#[tokio::test]
async fn participants_and_methods_are_listed() {
    let server = instances_server(
        format!("{}{}", test_module("t", &[]), instance("t-1", "t")),
        &[],
    );
    let owner = server.owner().await;
    wait_up(&server, &owner, "t-1").await;
    let _outside = server.test_runtime("ws-1", &[]).await;

    // Participants: the ones that are not instances, with their client names.
    let reply = listing(&owner).await;
    let names: Vec<_> = reply
        .participants
        .iter()
        .map(|participant| {
            (
                participant.identity.as_str(),
                participant.client_name.as_str(),
            )
        })
        .collect();
    assert!(names.contains(&("owner", "owner-test")), "{names:?}");
    assert!(names.contains(&("owner", "ws-1")), "{names:?}");
    assert!(
        names
            .iter()
            .all(|(identity, _)| !identity.starts_with("instance:"))
    );
    let outside = reply
        .participants
        .iter()
        .find(|participant| participant.client_name == "ws-1")
        .unwrap();
    assert!(outside.provides.contains(&"test.echo".to_owned()));
    assert_eq!(outside.address, "unix");
    let t1 = reply
        .instances
        .iter()
        .find(|info| info.name == "t-1")
        .unwrap();
    assert!(t1.provides.contains(&"test.echo".to_owned()));

    // Methods: each with its kind, lane and providers in connection order; tilrd's own too.
    let methods = method_list(&owner).await;
    let provider_of = |name: &str| {
        methods
            .methods
            .iter()
            .find(|info| info.method == name)
            .unwrap_or_else(|| panic!("{name} is not listed"))
    };
    assert_eq!(
        provider_of("test.echo").providers,
        ["instance:t-1", "owner"]
    );
    let count = provider_of("test.count");
    assert_eq!(count.kind, tilr_bus::bus::Kind::Stream as i32);
    assert_eq!(count.lane, Lane::Interactive as i32);
    assert_eq!(provider_of("bus.ping").providers, ["tilrd"]);
    assert_eq!(provider_of("bus.instances").providers, ["tilrd"]);

    // Draining is not offered: while t-1 waits for a call it is not a provider. The call
    // takes longer than the drain, so t-1 drains until the call is dropped below.
    let mut sleeping = Box::pin(owner.call("test.sleep", "60000", to("t-1")));
    let _ = sleeping.as_mut().now_or_never();
    calls(&owner, "test.echo", "sync").await.unwrap();
    let reloading = {
        let other = server.owner().await;
        tokio::spawn(async move { reload(&other, What::Instance("t-1".into())).await })
    };
    let draining = wait_instance(&server, &owner, "t-1", "draining", |info| {
        info.state == "draining"
    })
    .await;
    assert!(draining.pid > 0);
    let methods = method_list(&owner).await;
    let echo = methods
        .methods
        .iter()
        .find(|info| info.method == "test.echo")
        .unwrap();
    assert_eq!(echo.providers, ["owner"], "{echo:?}");
    assert!(
        methods
            .methods
            .iter()
            .all(|info| !info.providers.contains(&"instance:t-1".to_owned()))
    );
    // Dropped, the call is cancelled: the drain ends and the reload goes on.
    drop(sleeping);
    bounded("the reload", reloading).await.unwrap().unwrap();
    wait_up(&server, &owner, "t-1").await;
}

#[tokio::test]
async fn an_instances_output_is_in_the_server_log() {
    let server = instances_server(
        format!(
            "{}{}",
            script_module(
                "talker",
                "echo hello stdout; echo hello log >&2; exec sleep 3600",
                &[]
            ),
            instance_with("t-1", "talker", "", &[], &[])
        ),
        &[],
    );
    server.wait_logged("bus t-1: hello log").await;
    server.wait_logged("bus t-1: hello stdout").await;
    let lines = server.log_lines();
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("bus t-1: hello log")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.ends_with("bus t-1: hello stdout"))
    );
}

#[tokio::test]
async fn every_transport_speaks_the_same_envelopes() {
    let server = instances_server(
        format!(
            "{}{}{}",
            test_module("t", &[]),
            instance("t-1", "t"),
            instance_with(
                "t-2",
                "t",
                "connect = \"socket\"\n",
                &[("provide", &["test.*"])],
                &[]
            )
        ),
        &[],
    );
    let owner = server.owner().await;
    wait_up(&server, &owner, "t-1").await;
    wait_up(&server, &owner, "t-2").await;
    // The owner on the Unix socket, as a WebSocket participant that provides too.
    let _ws = server.test_runtime("ws-1", &[]).await;

    // The same call over descriptor 3, an instance socket and a WebSocket.
    for target in ["instance:t-1", "instance:t-2", "owner"] {
        let reply = bounded(
            target,
            owner.call("test.echo", "x", CallOptions::default().target(target)),
        )
        .await
        .unwrap_or_else(|error| panic!("{target}: {error}"));
        assert_eq!(text(&reply.payload), "x", "{target}");
        // A JSON payload keeps its content type on each.
        let reply = bounded(
            target,
            owner.call(
                "test.echo",
                "{\"a\":1}",
                CallOptions::default()
                    .target(target)
                    .content_type(tilr_bus::CONTENT_TYPE_JSON),
            ),
        )
        .await
        .unwrap();
        assert_eq!(reply.content_type, tilr_bus::CONTENT_TYPE_JSON, "{target}");
    }
    let t1 = info(&owner, "t-1").await;
    let t2 = info(&owner, "t-2").await;
    assert_eq!(
        (t1.transport.as_str(), t2.transport.as_str()),
        ("fd", "socket")
    );
}

#[tokio::test]
async fn a_frame_over_the_limit_closes_the_connection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let tokens = std::env::temp_dir().join(format!("tilr-bus-frame-{}", std::process::id()));
    let _ = std::fs::remove_file(&tokens);
    let server = instances_server(
        format!(
            "{}{}",
            token_module("s", &tokens),
            instance_with("s-1", "s", "connect = \"socket\"\n", &[], &[])
        ),
        &[],
    );
    let owner = server.owner().await;
    let first = wait_tokens(&server, "s-1", &tokens, 1).await;

    // The instance's side by hand: a length prefix, an envelope.
    let mut stream = tokio::net::UnixStream::connect(instance_socket(&server, "s-1"))
        .await
        .unwrap();
    let hello = tilr_bus::bus::Envelope {
        msg: Some(Msg::Hello(Hello {
            protocol_version: tilr_bus::PROTOCOL_VERSION,
            client_name: "by-hand".into(),
            instance_token: first[0].clone(),
            ..Hello::default()
        })),
    };
    stream
        .write_all(&tilr_bus::codec::encode_stream(&hello).unwrap())
        .await
        .unwrap();
    let mut buf = bytes::BytesMut::new();
    let ack = loop {
        if let Some(envelope) = tilr_bus::codec::decode_stream(&mut buf).unwrap() {
            break envelope;
        }
        let mut chunk = [0u8; 4096];
        let read = tokio::time::timeout(common::TIMEOUT, stream.read(&mut chunk))
            .await
            .expect("HelloAck")
            .unwrap();
        assert!(read > 0, "closed before HelloAck");
        buf.extend_from_slice(&chunk[..read]);
    };
    let Some(Msg::HelloAck(ack)) = ack.msg else {
        panic!("not a HelloAck: {ack:?}");
    };
    assert_eq!(ack.identity, "instance:s-1");
    assert_eq!(ack.max_frame as usize, tilr_bus::codec::MAX_FRAME);
    let up = wait_up(&server, &owner, "s-1").await;

    // One byte over the limit in the length prefix: the connection ends, the process
    // is killed and started again.
    let over = (tilr_bus::codec::MAX_FRAME as u32 + 1).to_be_bytes();
    stream.write_all(&over).await.unwrap();
    let mut rest = Vec::new();
    let closed = tokio::time::timeout(common::TIMEOUT, stream.read_to_end(&mut rest))
        .await
        .expect("the connection is closed");
    let _ = closed;
    let again = wait_tokens(&server, "s-1", &tokens, 2).await;
    assert_ne!(again[0], again[1]);
    let now = wait_instance(&server, &owner, "s-1", "a new process", |info| {
        info.pid > 0 && info.pid != up.pid && info.restarts == 1
    })
    .await;
    assert!(!alive(up.pid) && alive(now.pid));
    let _ = std::fs::remove_file(&tokens);
}

/// The instance case of `reserved_namespaces_cannot_be_provided`: an instance that
/// provides a method of a reserved namespace is refused with `reserved`; it is not an
/// instance that is up, and it is started again with a pause like a crashed one.
async fn a_reserved_method_from_an_instance() {
    let tokens = std::env::temp_dir().join(format!("tilr-bus-reserved-{}", std::process::id()));
    let _ = std::fs::remove_file(&tokens);
    let server = instances_server(
        format!(
            "{}{}",
            token_module("s", &tokens),
            instance_with(
                "s-1",
                "s",
                "connect = \"socket\"\n",
                &[("provide", &["pane.*", "test.*"])],
                &[]
            )
        ),
        &[],
    );
    let owner = server.owner().await;
    let first = wait_tokens(&server, "s-1", &tokens, 1).await;
    let refused = tilr_bus::Bus::connect(
        tilr_bus::Transport::Socket {
            path: instance_socket(&server, "s-1"),
            token: first[0].clone(),
        },
        ConnectOptions::new("pretender").provide(
            MethodSpec::unary("pane.create"),
            Handler::unary(|_: Request| async { Ok("no") }),
        ),
    )
    .await
    .expect_err("a reserved method is refused");
    assert_eq!(refused.code, codes::RESERVED, "{refused}");
    // Refused at Hello: the instance did not come up, and it is started again.
    let again = wait_tokens(&server, "s-1", &tokens, 2).await;
    assert_ne!(again[0], again[1]);
    let now = wait_instance(&server, &owner, "s-1", "a restart", |info| {
        info.restarts >= 1
    })
    .await;
    assert_ne!(now.state, "up");
    let _ = std::fs::remove_file(&tokens);
}

#[tokio::test]
async fn a_reload_stops_instances_and_the_successor_starts_them() {
    let server = instances_server(
        format!(
            "{}{}{}",
            test_module("t", &[]),
            instance("t-1", "t"),
            instance("t-2", "t")
        ),
        &[],
    );
    let owner = server.owner().await;
    let t1 = wait_up(&server, &owner, "t-1").await;
    let t2 = wait_up(&server, &owner, "t-2").await;
    // Stopped by hand: until the handover of `bus-reload` the successor reads `bus.toml`
    // and starts what it says.
    stop_instance(&owner, "t-2", None).await.unwrap();
    server.reload().await;
    assert!(!alive(t1.pid), "the instance did not survive the exec");
    drop(owner);
    let owner = server.owner().await;
    for (name, before) in [("t-1", t1.pid), ("t-2", t2.pid)] {
        let after = wait_instance(&server, &owner, name, "a new process", |info| {
            info.state == "up" && info.pid != before
        })
        .await;
        assert_eq!(after.restarts, 0, "{name}");
    }
    assert_eq!(calls(&owner, "test.echo", "x").await.unwrap(), "x");
}

#[tokio::test]
async fn a_reload_of_the_server_does_not_wait_for_a_long_drain() {
    let server = instances_server(
        format!("{}{}", test_module("t", &[]), instance("t-1", "t")),
        &[],
    );
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "t-1").await;
    // A drain is at most 30 s.
    let error = stop_instance(&owner, "t-1", Some(30_001))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST, "{error}");
    assert!(error.message.contains("30000"), "{error}");

    // A call that takes a minute, and a stop that drains it for up to 30 s.
    let mut sleeping = Box::pin(owner.call(
        "test.sleep",
        "60000",
        to("t-1").timeout(Duration::from_secs(60)),
    ));
    let _ = sleeping.as_mut().now_or_never();
    calls(&owner, "test.echo", "sync").await.unwrap();
    let stopper = server.owner().await;
    let stopping = tokio::spawn(async move { stop_instance(&stopper, "t-1", Some(30_000)).await });
    wait_instance(&server, &owner, "t-1", "draining", |info| {
        info.state == "draining"
    })
    .await;

    // The server reloads: the drain ends at once, the instance is stopped and the
    // successor serves (it starts the instance again from bus.toml).
    let began = std::time::Instant::now();
    server.reload().await;
    assert!(
        began.elapsed() < Duration::from_secs(15),
        "the reload waited for the drain: {:?}",
        began.elapsed()
    );
    assert!(!alive(before.pid));
    let ended = tokio::time::timeout(common::TIMEOUT, sleeping)
        .await
        .expect("the call in flight ends with the reload");
    assert!(ended.is_err(), "{ended:?}");
    stopping.abort();
    let owner = server.owner().await;
    let after = wait_up(&server, &owner, "t-1").await;
    assert_ne!(after.pid, before.pid);
}

#[tokio::test]
async fn a_failed_reload_starts_the_instances_again() {
    let server = BusServer::start(BusOptions {
        bus_toml: Some(format!("{}{}", test_module("t", &[]), instance("t-1", "t"))),
        failpoints: "reload-before-exec=error".into(),
        ..BusOptions::default()
    });
    let owner = server.owner().await;
    let before = wait_up(&server, &owner, "t-1").await;
    server.signal("-USR1");
    server.wait_logged("reload failed").await;
    // Stopped before the exec, which did not happen: started again by this server.
    let after = wait_instance(&server, &owner, "t-1", "a new process", |info| {
        info.state == "up" && info.pid != before.pid
    })
    .await;
    assert!(!alive(before.pid));
    assert_eq!(calls(&owner, "test.echo", "x").await.unwrap(), "x");
    assert_eq!(after.restarts, 0);
}

#[tokio::test]
async fn the_control_methods_answer_json() {
    let server = instances_server(
        format!("{}{}", test_module("t", &[]), instance("t-1", "t")),
        &[],
    );
    let owner = server.owner().await;
    wait_up(&server, &owner, "t-1").await;
    let json = |method: &'static str, body: &'static str| {
        let owner = &owner;
        async move {
            let reply = bounded(
                method,
                owner.call(
                    method,
                    body,
                    CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
                ),
            )
            .await?;
            assert_eq!(reply.content_type, tilr_bus::CONTENT_TYPE_JSON, "{method}");
            Ok::<_, BusError>(
                serde_json::from_str::<serde_json::Value>(&text(&reply.payload)).unwrap(),
            )
        }
    };

    let listed = json("bus.instances", "{}").await.unwrap();
    let t1 = &listed["instances"][0];
    assert_eq!(
        (t1["name"].as_str(), t1["state"].as_str()),
        (Some("t-1"), Some("up"))
    );
    assert!(t1["pid"].as_u64().unwrap() > 0);
    assert!(
        listed["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["identity"] == "owner"
                && p["clientName"] == "owner-test"
                && p["connectionId"].is_string())
    );
    let methods = json("bus.methods", "").await.unwrap();
    let echo = methods["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["method"] == "test.echo")
        .unwrap();
    assert_eq!(
        (
            echo["kind"].as_str(),
            echo["lane"].as_str(),
            echo["select"].as_str()
        ),
        (Some("UNARY"), Some("INTERACTIVE"), Some("ROUND_ROBIN"))
    );
    assert_eq!(echo["providers"], serde_json::json!(["instance:t-1"]));

    json("bus.stop", "{\"instance\": \"t-1\", \"drain_ms\": 0}")
        .await
        .unwrap();
    assert_eq!(info(&owner, "t-1").await.state, "stopped");
    let started = json("bus.start", "{\"instance\": \"t-1\"}").await.unwrap();
    assert_eq!(
        started["pid"].as_u64().unwrap() as u32,
        info(&owner, "t-1").await.pid
    );
    let reloaded = json("bus.reload", "{\"instance\": \"t-1\"}").await.unwrap();
    let done = &reloaded["reloaded"][0];
    assert_eq!(done["instance"], "t-1");
    assert_eq!(done["oldPid"], started["pid"]);
    assert_eq!(
        done["newPid"].as_u64().unwrap() as u32,
        info(&owner, "t-1").await.pid
    );
    let error = json("bus.reload", "{}").await.unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST, "{error}");
    let error = json("bus.start", "[1]").await.unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST, "{error}");
    let config = json("bus.reload_config", "{}").await.unwrap();
    assert_eq!(config["kept"], serde_json::json!(["t-1"]));
}
