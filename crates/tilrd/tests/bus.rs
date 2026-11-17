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
