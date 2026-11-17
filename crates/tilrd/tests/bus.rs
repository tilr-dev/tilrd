//! Use cases: the bus (`tilr.bus.v1`) beside `tilr.v1`: Hello, the broker's
//! `from`, routing, order, streams and events. Participants are the test runtime
//! and `tilr_bus` clients as the owner on the Unix socket, told apart by their
//! client names, and raw clients that write envelopes by hand.
//!
//! Each change appends its tests as one block at the end of this file.

mod common;

use std::time::Duration;

use bytes::Bytes;
use common::bus::{BusOptions, BusServer, TcpAuth, text};
use common::bus_streams::within;
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
    within(bus.call(method, payload.to_owned(), CallOptions::default()))
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
    let mut events = within(subscriber.subscribe("test.*")).await.unwrap();

    // Meanwhile a pane is created (tilr.v1) and events are emitted.
    let mut v1 = Client::connect(&server).await;
    v1.open_pane().await;
    within(subscriber.emit("test.a", "before")).await.unwrap();
    assert_eq!(within(events.next()).await.unwrap().subject, "test.a");

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
    let pong: PingReply = within(bus.call_typed(
        "bus.ping",
        &PingRequest { nonce: 42 },
        CallOptions::default(),
    ))
    .await
    .unwrap();
    assert_eq!(pong.nonce, 42);
    let json = within(bus.call(
        "bus.ping",
        r#"{"nonce":"7"}"#,
        CallOptions::default().content_type(tilr_bus::CONTENT_TYPE_JSON),
    ))
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

    // A method nobody provides is not_found (bus-streams-events: unknown and
    // gone are told apart, `a_failed_call_says_why`); a target that is not
    // connected is unavailable.
    let error = owner_calls(&owner, "nobody.here", "").await.unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND);
    assert!(error.message.contains("nobody.here"), "{error:?}");
    let error = within(owner.call(
        "test.whoami",
        Bytes::new(),
        CallOptions::default().target("instance:t-9"),
    ))
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
    within(bus.activate()).await.unwrap();
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
async fn a_hello_provides_at_most_1024_methods() {
    let server = start();
    let mut options = ConnectOptions::new("hoarder");
    for i in 0..1025 {
        options = options.provide(
            MethodSpec::unary(format!("test.m{i}")),
            Handler::unary(|_: Request| async { Ok("no") }),
        );
    }
    let error = server.try_owner_with(options).await.unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST, "{error}");
    assert!(error.message.contains("1025 methods"), "{error}");
    // 1024 are fine.
    let mut options = ConnectOptions::new("provider");
    for i in 0..1024 {
        options = options.provide(
            MethodSpec::unary(format!("test.m{i}")),
            Handler::unary(|_: Request| async { Ok("yes") }),
        );
    }
    let provider = server.try_owner_with(options).await.unwrap();
    drop(provider);
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
    let mut events = within(owner.subscribe("pane.*")).await.unwrap();
    within(owner.emit("pane.created", "forged")).await.unwrap();
    let mut probe = within(owner.subscribe("probe.*")).await.unwrap();
    within(owner.emit("probe.x", "")).await.unwrap();
    // The client hands events to its subscriptions in the order they came, so a
    // forged pane.created would be waiting by the time probe.x is.
    assert_eq!(within(probe.next()).await.unwrap().subject, "probe.x");
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
    // In batches of 250: a caller has at most 256 calls pending (B5).
    const BATCH: u64 = 250;
    let mut seen = Vec::new();
    for batch in (0..CALLS).step_by(BATCH as usize) {
        for i in batch..batch + BATCH {
            t1.send(call(i + 1, "test.record", &i.to_string())).await;
        }
        while seen.len() < (batch + BATCH) as usize {
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
        let mut replies = 0;
        while replies < BATCH {
            if let Some(Msg::Reply(reply)) = t1.recv().await {
                replies += 1;
                assert!(matches!(reply.result, Some(reply::Result::Payload(_))));
            }
        }
    }
    let expected: Vec<String> = (0..CALLS).map(|i| i.to_string()).collect();
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn a_stream_is_a_call_answered_in_chunks() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    let owner = server.owner().await;

    // Count to three.
    let mut stream = within(owner.stream("test.count", "3", tilr_bus::StreamOptions::default()))
        .await
        .unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = within(stream.next()).await {
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
    let mut star = within(t2.subscribe("test.*")).await.unwrap();
    let mut tail = within(t3.subscribe("test.>")).await.unwrap();

    within(t1.emit("test.a", "x")).await.unwrap();
    within(t1.emit("test.a.b", "y")).await.unwrap();
    within(t1.emit("test.end", "")).await.unwrap();

    let event = within(star.next()).await.unwrap();
    assert_eq!(
        (event.subject.as_str(), event.from.as_str()),
        ("test.a", "owner")
    );
    assert_eq!(text(&event.payload), "x");
    // test.a.b is not one segment: the next is the end marker.
    assert_eq!(within(star.next()).await.unwrap().subject, "test.end");
    let subjects: Vec<String> = [
        within(tail.next()).await.unwrap(),
        within(tail.next()).await.unwrap(),
        within(tail.next()).await.unwrap(),
    ]
    .into_iter()
    .map(|event| event.subject)
    .collect();
    assert_eq!(subjects, ["test.a", "test.a.b", "test.end"]);

    // No history: a later subscriber gets only later events.
    within(t1.emit("test.a", "old")).await.unwrap();
    let t4 = server.owner().await;
    let mut late = within(t4.subscribe("test.*")).await.unwrap();
    within(t1.emit("test.b", "new")).await.unwrap();
    let event = within(late.next()).await.unwrap();
    assert_eq!(
        (event.subject.as_str(), text(&event.payload).as_str()),
        ("test.b", "new")
    );

    // A subscriber with two matching patterns gets an event once.
    let mut also = within(t4.subscribe("test.>")).await.unwrap();
    within(t1.emit("test.c", "")).await.unwrap();
    within(t1.emit("test.end", "")).await.unwrap();
    assert_eq!(within(late.next()).await.unwrap().subject, "test.c");
    assert_eq!(within(late.next()).await.unwrap().subject, "test.end");
    assert_eq!(within(also.next()).await.unwrap().subject, "test.c");
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
                    // 800 KiB in all: within the stream's and the connection's
                    // window, which this caller never credits.
                    payload: Bytes::from(vec![i as u8; 8 << 10]),
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
// bus-streams-events
// ---------------------------------------------------------------------------

use common::bus_streams::{
    add, call_with, chunks_until_quiet, count, credit, error_of, events_dropped, expect,
};
use tilr_bus::bus::Subscribe;

const MIB: usize = 1 << 20;

#[tokio::test]
async fn a_failed_call_says_why() {
    let server = start();
    let owner = server.owner().await;

    // A method nobody ever provided.
    let error = owner_calls(&owner, "nobody.here", "").await.unwrap_err();
    assert_eq!(error.code, codes::NOT_FOUND);
    assert!(error.message.contains("nobody.here"), "{error:?}");

    // No window open, on a server no window ever reached: ui.notify is a
    // method of windows, unavailable.
    let error = owner_calls(&owner, "ui.notify", "hi").await.unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (codes::UNAVAILABLE, "no provider of ui.notify is connected")
    );

    // A window provided ui.notify and is gone: unavailable again (until the
    // broker has seen it go, the call reaches it and ends unavailable too).
    let window = server
        .owner_with(ConnectOptions::new("window").provide(
            MethodSpec::unary("ui.notify").with_select(Select::LastActive),
            Handler::unary(|_: Request| async { Ok("shown") }),
        ))
        .await;
    assert_eq!(
        owner_calls(&owner, "ui.notify", "hi").await.unwrap(),
        "shown"
    );
    window.close();
    let _ = tokio::time::timeout(TIMEOUT, window.closed()).await;
    let error = owner_calls(&owner, "ui.notify", "hi").await.unwrap_err();
    assert_eq!(error.code, codes::UNAVAILABLE, "{error:?}");

    // A name only a participant that is gone provided is forgotten with it: a
    // connection cannot fill the broker with names. The successor of the same
    // client id is greeted after its predecessor is gone, and its calls are
    // handled after its greeting.
    let server = &server;
    let junk = |provides| async move {
        let mut raw = server.raw().await;
        raw.send(Msg::Hello(Hello {
            protocol_version: tilr_bus::PROTOCOL_VERSION,
            client_name: "junk".into(),
            client_id: "junk-1".into(),
            provides,
            ..Hello::default()
        }))
        .await;
        assert!(matches!(raw.recv().await, Some(Msg::HelloAck(_))));
        raw
    };
    let _first = junk(vec![MethodSpec::unary("junk.name")]).await;
    let mut second = junk(Vec::new()).await;
    second.send(call(1, "junk.name", "")).await;
    let (_, code, message) = error_of(&second.recv().await.unwrap()).expect("an error");
    assert_eq!(code, codes::NOT_FOUND, "{message}");

    // No reply within the call's deadline: timeout naming the method, and the
    // callee is told with Cancel.
    let mut never = server
        .raw_greeted(
            "never",
            vec![
                MethodSpec::unary("test.never"),
                MethodSpec::stream("test.never_stream", Lane::Interactive),
            ],
        )
        .await;
    let mut raw = server.raw_greeted("raw", Vec::new()).await;
    raw.send(call_with(1, "test.never", "", 0, 200)).await;
    let (hop, timeout_ms) = never
        .wait_for(|msg| match msg {
            Msg::Call(call) => Some((call.id, call.timeout_ms)),
            _ => None,
        })
        .await;
    assert!(timeout_ms > 0 && timeout_ms <= 200, "{timeout_ms}");
    let (id, code, message) = error_of(&raw.recv().await.unwrap()).expect("an error reply");
    assert_eq!((id, code.as_str()), (1, codes::TIMEOUT));
    assert!(message.contains("test.never"), "{message}");
    let cancelled = never
        .wait_for(|msg| match msg {
            Msg::Cancel(cancel) => Some(cancel.id),
            _ => None,
        })
        .await;
    assert_eq!(cancelled, hop);

    // A unary call has a deadline even when it names none (call_timeout_ms):
    // the callee is told the time it has.
    raw.send(call_with(2, "test.never", "", 0, 0)).await;
    let timeout_ms = never
        .wait_for(|msg| match msg {
            Msg::Call(call) => Some(call.timeout_ms),
            _ => None,
        })
        .await;
    assert_eq!(timeout_ms, tilr_bus::limits::CALL_TIMEOUT_MS);

    // A stream window over 16 MiB.
    raw.send(call_with(
        3,
        "test.never_stream",
        "",
        tilr_bus::limits::MAX_STREAM_WINDOW + 1,
        0,
    ))
    .await;
    let (id, code, message) = error_of(&raw.recv().await.unwrap()).expect("an error reply");
    assert_eq!((id, code.as_str()), (3, codes::BAD_REQUEST));
    assert!(message.contains("test.never_stream"), "{message}");
}

#[tokio::test]
async fn a_full_queue_is_answered_busy() {
    let server = start();
    // A method that never replies; the caller sends 300 calls of it.
    let mut never = server
        .raw_greeted("never", vec![MethodSpec::unary("test.never")])
        .await;
    let mut t1 = server.raw_greeted("t-1", Vec::new()).await;
    const CALLS: u64 = 300;
    let in_flight = u64::from(tilr_bus::limits::MAX_IN_FLIGHT);
    for id in 1..=CALLS {
        t1.send(call(id, "test.never", "")).await;
    }
    for expected in in_flight + 1..=CALLS {
        let (id, code, message) =
            error_of(&t1.recv().await.unwrap()).expect("a busy reply for each call past 256");
        assert_eq!((id, code.as_str()), (expected, codes::BUSY));
        assert!(message.contains("test.never"), "{message}");
    }
    let mut hops = Vec::new();
    while hops.len() < in_flight as usize {
        if let Some(Msg::Call(call)) = never.recv().await {
            hops.push(call.id);
        }
    }
    // Pending: the callee answers them now (unless their 10 s ran out first on
    // a slow run: then timeout, never busy), and only those 256 came to it.
    for hop in &hops {
        never
            .send(Msg::Reply(WireReply {
                id: *hop,
                result: Some(reply::Result::Payload(Bytes::from_static(b"late"))),
                content_type: String::new(),
            }))
            .await;
    }
    let mut answered = std::collections::BTreeSet::new();
    while answered.len() < in_flight as usize {
        let msg = t1.recv().await.unwrap();
        let id = match &msg {
            Msg::Reply(reply) if error_of(&msg).is_none() => reply.id,
            _ => {
                let (id, code, _) = error_of(&msg).unwrap();
                assert_eq!(code, codes::TIMEOUT, "{msg:?}");
                id
            }
        };
        assert!(id <= in_flight && answered.insert(id), "{msg:?}");
    }
    for raw in [&mut never, &mut t1] {
        raw.send(call(0, "bus.ping", "")).await;
        loop {
            match raw.recv().await.expect("open") {
                Msg::Reply(reply) if reply.id == 0 => break,
                Msg::Cancel(_) => {}
                other => panic!("only 256 reach the callee, once each: {other:?}"),
            }
        }
    }

    // A callee that stops reading: once its queue is as full as calls may fill
    // it, more calls are answered busy, and it is not closed for it.
    let mut stuck = server
        .raw_greeted("stuck", vec![MethodSpec::unary("test.stuck")])
        .await;
    let payload = "x".repeat(4 << 10);
    let mut callers = Vec::new();
    for n in 0..6 {
        let mut caller = server.raw_greeted(&format!("caller-{n}"), Vec::new()).await;
        for id in 1..=in_flight {
            caller.send(call(id, "test.stuck", &payload)).await;
        }
        callers.push(caller);
    }
    let mut busy = 0;
    for caller in &mut callers {
        // A connection's envelopes are handled in order: the ping's reply comes
        // after every refusal of the calls before it (a call queued and timed
        // out on a slow run is answered timeout, never busy).
        caller.send(call(0, "bus.ping", "")).await;
        loop {
            let msg = caller.recv().await.expect("open");
            if matches!(&msg, Msg::Reply(reply) if reply.id == 0) {
                break;
            }
            let (_, code, message) = error_of(&msg).expect("only busy replies");
            if code == codes::TIMEOUT {
                continue;
            }
            assert_eq!(code, codes::BUSY);
            assert!(
                message.contains("test.stuck") && message.contains("queue is full"),
                "{message}"
            );
            busy += 1;
        }
    }
    assert!(busy > 0, "a full queue answers busy");
    // It reads again: the calls queued for it come (and the cancels of any that
    // timed out), not a close, and nothing else: its own ping's answer is next.
    let mut queued = 0;
    while queued < 6 * in_flight - busy {
        match stuck.recv().await.expect("not closed") {
            Msg::Call(_) => queued += 1,
            Msg::Cancel(_) => {}
            other => panic!("{other:?}"),
        }
    }
    stuck.send(call(0, "bus.ping", "")).await;
    loop {
        match stuck.recv().await.expect("not closed") {
            Msg::Reply(reply) if reply.id == 0 => break,
            Msg::Cancel(_) => {}
            other => panic!("each call queued or refused: {other:?}"),
        }
    }
    assert!(stuck.close.is_none(), "closed: {:?}", stuck.close);
}

#[tokio::test]
async fn a_nested_call_gets_the_time_that_is_left_and_a_cancel() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    let mut t2 = server
        .raw_greeted(
            "t-2",
            vec![
                MethodSpec::unary("test.deadline_seen"),
                MethodSpec::unary("test.hold"),
            ],
        )
        .await;
    let owner = server.owner().await;

    // t-1 waits 400 ms of the owner's 1000, then calls t-2 with what is left.
    let nested = tokio::spawn(async move {
        within(owner.call(
            "test.nested",
            "400 test.deadline_seen",
            CallOptions::default().timeout(Duration::from_millis(1000)),
        ))
        .await
    });
    let (hop, timeout_ms) = t2
        .wait_for(|msg| match msg {
            Msg::Call(call) if call.method == "test.deadline_seen" => {
                Some((call.id, call.timeout_ms))
            }
            _ => None,
        })
        .await;
    assert!(timeout_ms > 0 && timeout_ms <= 600, "{timeout_ms}");
    // t-2 never answers: at the deadline it is cancelled.
    let cancelled = t2
        .wait_for(|msg| match msg {
            Msg::Cancel(cancel) => Some(cancel.id),
            _ => None,
        })
        .await;
    assert_eq!(cancelled, hop);
    let error = within(nested).await.unwrap().unwrap_err();
    assert_eq!(error.code, codes::TIMEOUT);

    // A caller whose connection ends while its call runs.
    let mut caller = server.raw_greeted("caller", Vec::new()).await;
    caller.send(call(9, "test.hold", "")).await;
    let hop = t2
        .wait_for(|msg| match msg {
            Msg::Call(call) if call.method == "test.hold" => Some(call.id),
            _ => None,
        })
        .await;
    drop(caller);
    let cancelled = t2
        .wait_for(|msg| match msg {
            Msg::Cancel(cancel) => Some(cancel.id),
            _ => None,
        })
        .await;
    assert_eq!(cancelled, hop);
}

#[tokio::test]
async fn a_lagging_subscriber_loses_events_not_its_connection() {
    let server = start();
    let mut subscriber = server.raw_greeted("subscriber", Vec::new()).await;
    subscriber
        .send(Msg::Subscribe(Subscribe {
            id: 1,
            pattern: "test.*".into(),
        }))
        .await;
    subscriber
        .wait_for(|msg| matches!(msg, Msg::Reply(reply) if reply.id == 1).then_some(()))
        .await;

    // It stops reading while t-1 emits.
    let t1 = server.owner_with(ConnectOptions::new("t-1")).await;
    const EVENTS: u64 = 100_000;
    for i in 0..EVENTS {
        within(t1.emit("test.flood", i.to_string())).await.unwrap();
    }
    // Its calls are handled in order: once this is answered, every event was
    // delivered or dropped.
    owner_calls(&t1, "bus.ping", "").await.unwrap();

    // It reads again: what was queued for it, then a later event.
    let mut received = 0u64;
    while let Some(msg) = subscriber.recv_within(Duration::from_millis(500)).await {
        match msg {
            Msg::Event(event) if event.subject == "test.flood" => received += 1,
            other => panic!("{other:?}"),
        }
    }
    within(t1.emit("test.end", "")).await.unwrap();
    subscriber
        .wait_for(|msg| match msg {
            Msg::Event(event) if event.subject == "test.end" => Some(()),
            Msg::Event(event) if event.subject == "test.flood" => {
                received += 1;
                None
            }
            other => panic!("{other:?}"),
        })
        .await;
    assert!(received < EVENTS, "it lagged, yet got all {EVENTS}");
    assert!(subscriber.close.is_none());
    let dropped = events_dropped(&server, "test.flood").await;
    assert_eq!(
        received + dropped,
        EVENTS,
        "received {received}, dropped {dropped}"
    );
}

#[tokio::test]
async fn a_slow_participant_does_not_hold_up_the_others() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    // t-3 subscribes and provides, then stops reading; it is flooded with
    // events and calls meanwhile.
    let mut t3 = server
        .raw_greeted("t-3", vec![MethodSpec::unary("test.three")])
        .await;
    t3.send(Msg::Subscribe(Subscribe {
        id: 1,
        pattern: "test.*".into(),
    }))
    .await;
    t3.wait_for(|msg| matches!(msg, Msg::Reply(reply) if reply.id == 1).then_some(()))
        .await;
    let emitter = server.owner_with(ConnectOptions::new("emitter")).await;
    let flood = tokio::spawn(async move {
        for i in 0..20_000 {
            within(emitter.emit("test.flood", i.to_string()))
                .await
                .unwrap();
        }
        emitter
    });
    let mut caller = server.raw_greeted("caller", Vec::new()).await;
    let payload = "x".repeat(4 << 10);
    for id in 1..=200 {
        caller.send(call(id, "test.three", &payload)).await;
    }

    // Every call is answered while t-3 reads nothing at all: a broker that
    // waited on t-3 (a lock held over its queue, a full lane awaited) would
    // never answer them. Not timed: t-3 never reads until they are all done.
    let owner = server.owner().await;
    for i in 0..1000 {
        let echo = owner_calls(&owner, "test.echo", &i.to_string()).await;
        assert_eq!(echo.unwrap(), i.to_string());
    }
    drop(within(flood).await.unwrap());
    // t-3 was behind meanwhile: events for it were dropped.
    assert!(events_dropped(&server, "test.flood").await > 0);
    assert!(t3.close.is_none());
    drop(t3);
}

#[tokio::test]
async fn streams_are_paced_by_the_readers_credit() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    let mut owner = server.raw_greeted("owner-raw", Vec::new()).await;
    const WINDOW: usize = 64 << 10;
    owner
        .send(call_with(1, "test.flood", "", WINDOW as u32, 0))
        .await;
    let mut got = 0;
    while got < WINDOW {
        got += expect(
            &mut owner,
            |msg| match msg {
                Msg::Chunk(chunk) if chunk.id == 1 => Some(chunk.payload.len()),
                _ => None,
            },
            |_| false,
        )
        .await;
    }
    assert_eq!(got, WINDOW, "the window, then nothing");
    // Not credited: the stream stalls, the connection does not.
    owner.send(call(2, "test.echo", "ping")).await;
    let reply = expect(
        &mut owner,
        |msg| match msg {
            Msg::Reply(reply) if reply.id == 2 => Some(reply.clone()),
            _ => None,
        },
        |_| false,
    )
    .await;
    assert_eq!(
        reply.result,
        Some(reply::Result::Payload(Bytes::from_static(b"ping")))
    );
    assert!(
        chunks_until_quiet(&mut owner, Duration::from_millis(300))
            .await
            .is_empty()
    );
    // Credited: as much more comes.
    owner.send(credit(1, WINDOW)).await;
    let mut more = 0;
    while more < WINDOW {
        more += expect(
            &mut owner,
            |msg| match msg {
                Msg::Chunk(chunk) if chunk.id == 1 => Some(chunk.payload.len()),
                _ => None,
            },
            |_| false,
        )
        .await;
    }
    assert_eq!(more, WINDOW);
    assert!(
        chunks_until_quiet(&mut owner, Duration::from_millis(300))
            .await
            .is_empty()
    );
    owner
        .send(Msg::Cancel(Cancel {
            id: 1,
            reason: "enough".into(),
        }))
        .await;
}

#[tokio::test]
async fn a_stream_that_overruns_its_credit_is_ended() {
    let server = start();
    for (method, lane) in [
        ("test.overrun", Lane::Interactive),
        ("test.overrun_bulk", Lane::Bulk),
    ] {
        let mut provider = server
            .raw_greeted("overrun", vec![MethodSpec::stream(method, lane)])
            .await;
        let mut caller = server.raw_greeted("caller", Vec::new()).await;
        caller.send(call(1, method, "")).await;
        let (hop, window) = provider
            .wait_for(|msg| match msg {
                Msg::Call(call) => Some((call.id, call.window)),
                _ => None,
            })
            .await;
        assert_eq!(window as usize, MIB, "{method}: the default window");
        // 2 MiB, credit or not.
        for _ in 0..32 {
            provider
                .send(Msg::Chunk(Chunk {
                    id: hop,
                    payload: Bytes::from(vec![7; 64 << 10]),
                    content_encoding: String::new(),
                }))
                .await;
        }
        let mut got = 0;
        let error = loop {
            match caller.recv().await {
                Some(Msg::Chunk(chunk)) => got += chunk.payload.len(),
                Some(msg) => break error_of(&msg).expect("an error reply"),
                None => panic!("{method}: closed"),
            }
        };
        assert!(got <= MIB, "{method}: {got} bytes on a window of 1 MiB");
        assert_eq!((error.0, error.1.as_str()), (1, codes::BAD_REPLY));
        assert!(error.2.contains(method), "{}", error.2);
        let cancelled = provider
            .wait_for(|msg| match msg {
                Msg::Cancel(cancel) => Some(cancel.id),
                _ => None,
            })
            .await;
        assert_eq!(cancelled, hop);
    }
}

#[tokio::test]
async fn bulk_data_in_the_network_is_bounded_per_connection() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    // Two bulk reads of 8 MiB with windows of 4 MiB, and an interactive stream,
    // none credited.
    for id in [1, 2] {
        reader
            .send(call_with(id, "test.bulk", "8388608", 4 << 20, 0))
            .await;
    }
    const FLOOD: usize = 64 << 10;
    reader
        .send(call_with(3, "test.flood", "", FLOOD as u32, 0))
        .await;
    let mut seen = Vec::new();
    while count(&seen, 1) + count(&seen, 2) < MIB || count(&seen, 3) < FLOOD {
        match reader.recv().await {
            Some(Msg::Chunk(chunk)) => add(&mut seen, chunk.id, chunk.payload.len()),
            other => panic!("{other:?}"),
        }
        assert!(count(&seen, 1) + count(&seen, 2) <= MIB, "{seen:?}");
    }
    for (id, bytes) in chunks_until_quiet(&mut reader, Duration::from_millis(300)).await {
        add(&mut seen, id, bytes);
    }
    assert_eq!(count(&seen, 1) + count(&seen, 2), MIB, "{seen:?}");
    assert_eq!(count(&seen, 3), FLOOD, "{seen:?}");

    // Credited what it read: as much again, no more.
    for id in [1, 2] {
        if count(&seen, id) > 0 {
            reader.send(credit(id, count(&seen, id))).await;
        }
    }
    let before = count(&seen, 1) + count(&seen, 2);
    let mut more = 0;
    while more < MIB {
        match reader.recv().await {
            Some(Msg::Chunk(chunk)) => {
                assert!(chunk.id == 1 || chunk.id == 2, "{chunk:?}");
                more += chunk.payload.len();
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(
        chunks_until_quiet(&mut reader, Duration::from_millis(300))
            .await
            .is_empty()
    );
    assert_eq!(before + more, 2 * MIB);
}

#[tokio::test]
async fn interactive_traffic_goes_ahead_of_bulk_traffic() {
    let server = start();
    let _t1 = server.test_runtime("t-1", &[]).await;
    // A reader that credits each chunk it reads, and while an echo is on its
    // way credits nothing: whatever bulk data comes before the echo's answer
    // was sent before it, and no more than the next megabyte (the connection
    // window) may be.
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    reader.send(call(1, "test.bulk", "")).await;
    let (mut total, mut next_echo, mut echo_id, mut worst) = (0, MIB, 100, 0);
    loop {
        match reader.recv().await.expect("open") {
            Msg::Chunk(chunk) => {
                total += chunk.payload.len();
                reader.send(credit(1, chunk.payload.len())).await;
            }
            Msg::End(_) => break,
            other => panic!("{other:?}"),
        }
        if total < next_echo {
            continue;
        }
        next_echo += MIB;
        echo_id += 1;
        let sent = total.to_string();
        reader.send(call(echo_id, "test.echo", &sent)).await;
        let (mut behind, mut ended) = (0, false);
        loop {
            match reader.recv().await.expect("open") {
                Msg::Chunk(chunk) => behind += chunk.payload.len(),
                Msg::End(_) => ended = true,
                Msg::Reply(reply) if reply.id == echo_id => {
                    assert_eq!(reply.result, Some(reply::Result::Payload(sent.into())));
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(
            behind <= MIB,
            "an echo was answered after {behind} bytes of bulk data"
        );
        worst = worst.max(behind);
        total += behind;
        if ended {
            break;
        }
        if behind > 0 {
            reader.send(credit(1, behind)).await;
        }
    }
    assert_eq!(total, 64 * MIB, "the whole stream (worst wait {worst})");
}

use common::bus_streams::{chunk, noise_chunk, ping, send_within_credit};

/// The next `Call` a provider gets: its hop, method and window.
async fn next_call(provider: &mut common::bus::Raw) -> (u64, String, usize) {
    provider
        .wait_for(|msg| match msg {
            Msg::Call(call) => Some((call.id, call.method.clone(), call.window as usize)),
            _ => None,
        })
        .await
}

/// Wait until `provider` is told its stream `hop` is over.
async fn cancelled(provider: &mut common::bus::Raw, hop: u64) {
    provider
        .wait_for(|msg| matches!(msg, Msg::Cancel(cancel) if cancel.id == hop).then_some(()))
        .await;
}

/// Read a stream's chunks, crediting each, up to its end: the bytes and the end.
async fn read_crediting(reader: &mut common::bus::Raw, id: u64) -> (usize, Msg) {
    let mut bytes = 0;
    loop {
        match reader.recv().await.expect("open") {
            Msg::Chunk(chunk) => {
                assert_eq!(chunk.id, id);
                bytes += chunk.payload.len();
                reader.send(credit(id, chunk.payload.len())).await;
            }
            other => return (bytes, other),
        }
    }
}

#[tokio::test]
async fn the_end_of_a_relayed_bulk_stream_waits_behind_held_chunks() {
    let server = start();
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    for (id, how) in [(1, "end"), (2, "error"), (3, "timeout"), (4, "gone")] {
        let mut provider = server
            .raw_greeted("held", vec![MethodSpec::stream("test.held", Lane::Bulk)])
            .await;
        let timeout_ms = if how == "timeout" { 1000 } else { 0 };
        reader
            .send(call_with(id, "test.held", "", 4 << 20, timeout_ms))
            .await;
        let (hop, _, window) = next_call(&mut provider).await;
        assert_eq!(window, MIB, "the relay forwards at most 1 MiB");
        // 2 MiB not read yet: the connection window takes 1 MiB, the broker
        // holds the other, and the end waits behind it.
        let sent = send_within_credit(&mut provider, hop, window, 2 * MIB, 64 << 10).await;
        match how {
            "end" | "error" => {
                assert_eq!(sent, 2 * MIB, "{how}");
                provider
                    .send(if how == "end" {
                        Msg::End(End {
                            id: hop,
                            payload: Bytes::from_static(b"done"),
                        })
                    } else {
                        Msg::Reply(WireReply {
                            id: hop,
                            result: Some(reply::Result::Error(tilr_bus::bus::Error {
                                code: "test_failed".into(),
                                message: "after its chunks".into(),
                            })),
                            content_type: String::new(),
                        })
                    })
                    .await;
                ping(&mut provider, 99).await;
            }
            "timeout" => cancelled(&mut provider, hop).await,
            _ => drop(provider),
        }
        let (bytes, end) = read_crediting(&mut reader, id).await;
        let code = error_of(&end).map(|(_, code, _)| code);
        match how {
            "end" => {
                assert!(
                    matches!(&end, Msg::End(end) if &end.payload[..] == b"done"),
                    "{end:?}"
                )
            }
            "error" => assert_eq!(code.as_deref(), Some("test_failed")),
            "timeout" => assert_eq!(code.as_deref(), Some(codes::TIMEOUT)),
            _ => assert_eq!(code.as_deref(), Some(codes::UNAVAILABLE)),
        }
        // What the provider sent before its end, all of it, then the end (gone:
        // what reached the broker before it saw the connection end).
        if how == "timeout" || how == "gone" {
            assert!(bytes <= sent, "{bytes} of {sent}");
        } else {
            assert_eq!(bytes, 2 * MIB, "{how}");
        }
    }
}

#[tokio::test]
async fn a_bulk_stream_that_ends_uncredited_gives_the_window_back() {
    let server = start();
    let mut provider = server
        .raw_greeted("bulk", vec![MethodSpec::stream("test.bulk", Lane::Bulk)])
        .await;
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    for (id, bytes) in [(1, MIB), (2, 64 << 10)] {
        reader.send(call(id, "test.bulk", "")).await;
        let (hop, _, window) = next_call(&mut provider).await;
        assert_eq!(
            send_within_credit(&mut provider, hop, window, bytes, 64 << 10).await,
            bytes
        );
        provider
            .send(Msg::End(End {
                id: hop,
                payload: Bytes::new(),
            }))
            .await;
        // Read, never credited: the first stream spends the whole connection
        // window, which comes back with its end.
        let mut got = 0;
        loop {
            match reader.recv().await.expect("open") {
                Msg::Chunk(chunk) => got += chunk.payload.len(),
                Msg::End(end) => {
                    assert_eq!(end.id, id);
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(got, bytes);
    }
}

#[tokio::test]
async fn late_answers_after_a_timeout_or_a_cancel_are_dropped() {
    let server = start();
    let mut provider = server
        .raw_greeted(
            "late",
            vec![
                MethodSpec::unary("test.late"),
                MethodSpec::stream("test.late_stream", Lane::Interactive),
            ],
        )
        .await;
    let mut caller = server.raw_greeted("caller", Vec::new()).await;
    // A call that times out, a stream the caller cancels.
    caller.send(call_with(1, "test.late", "", 0, 100)).await;
    let (unary, _, _) = next_call(&mut provider).await;
    let (_, code, _) = error_of(&caller.recv().await.unwrap()).expect("an error");
    assert_eq!(code, codes::TIMEOUT);
    cancelled(&mut provider, unary).await;
    caller.send(call(2, "test.late_stream", "")).await;
    let (stream, _, _) = next_call(&mut provider).await;
    caller
        .send(Msg::Cancel(Cancel {
            id: 2,
            reason: "enough".into(),
        }))
        .await;
    cancelled(&mut provider, stream).await;
    // Their answers, late.
    provider
        .send(Msg::Reply(WireReply {
            id: unary,
            result: Some(reply::Result::Payload(Bytes::from_static(b"late"))),
            content_type: String::new(),
        }))
        .await;
    provider.send(chunk(stream, 10)).await;
    provider
        .send(Msg::End(End {
            id: stream,
            payload: Bytes::new(),
        }))
        .await;
    ping(&mut provider, 99).await;
    // None reached the caller: the next thing it hears is its own ping's answer.
    caller.send(call(3, "bus.ping", "")).await;
    let next = caller.recv().await.unwrap();
    assert!(
        matches!(&next, Msg::Reply(reply) if reply.id == 3),
        "{next:?}"
    );
}

#[tokio::test]
async fn an_empty_chunk_is_relayed_and_a_flood_of_tiny_ones_ends_the_stream() {
    let server = start();
    let mut provider = server
        .raw_greeted("tiny", vec![MethodSpec::stream("test.tiny", Lane::Bulk)])
        .await;
    let mut reader = server.raw_greeted("reader", Vec::new()).await;

    // An empty chunk is a chunk: it reaches the reader, behind it the rest.
    reader.send(call(1, "test.tiny", "")).await;
    let (hop, _, _) = next_call(&mut provider).await;
    provider.send(chunk(hop, 0)).await;
    provider.send(chunk(hop, 1)).await;
    provider
        .send(Msg::End(End {
            id: hop,
            payload: Bytes::new(),
        }))
        .await;
    let mut sizes = Vec::new();
    loop {
        match reader.recv().await.expect("open") {
            Msg::Chunk(chunk) if chunk.id == 1 => sizes.push(chunk.payload.len()),
            Msg::End(end) if end.id == 1 => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(sizes, [0, 1]);

    // Empty chunks to a reader that does not read: they use no credit, but the
    // broker counts each as 1 KiB of what it may hold, and ends the stream.
    reader.send(call(2, "test.tiny", "")).await;
    let (hop, _, _) = next_call(&mut provider).await;
    // The broker's RELAY_BUDGET (16 MiB) over CHUNK_COST (1 KiB) is 16384.
    let most = 4 * 16384;
    let mut sent = 0;
    'flood: while sent < most {
        for _ in 0..256 {
            provider.send(chunk(hop, 0)).await;
            sent += 1;
        }
        while let Some(msg) = provider.recv_within(Duration::ZERO).await {
            if matches!(msg, Msg::Cancel(cancel) if cancel.id == hop) {
                break 'flood;
            }
        }
    }
    assert!(sent < most, "all {sent} empty chunks were taken");
    let mut got = 0;
    let end = loop {
        match reader.recv().await.expect("open") {
            Msg::Chunk(_) => got += 1,
            other => break other,
        }
    };
    let (id, code, message) = error_of(&end).expect("an error");
    assert_eq!((id, code.as_str()), (2, codes::BAD_REPLY), "{message}");
    assert!(got < sent, "{got} of {sent}");
    // The reader is still served.
    ping(&mut reader, 3).await;
}

#[tokio::test]
async fn a_reader_that_stops_reading_large_interactive_chunks_is_closed_too_slow() {
    let server = start();
    let mut provider = server
        .raw_greeted(
            "big",
            vec![MethodSpec::stream("test.big", Lane::Interactive)],
        )
        .await;
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    const STREAMS: usize = 3;
    let window = tilr_bus::limits::MAX_STREAM_WINDOW;
    for id in 1..=STREAMS as u64 {
        reader.send(call_with(id, "test.big", "", window, 0)).await;
        let (hop, _, forwarded) = next_call(&mut provider).await;
        assert_eq!(forwarded, window as usize);
        // Its whole window in chunks of 1 MiB that do not compress, unread.
        for _ in 0..forwarded / MIB {
            provider.send(noise_chunk(hop, MIB)).await;
        }
    }
    ping(&mut provider, 99).await;
    // Far fewer than 1024 envelopes, but 48 MiB: closed 4001 before all came.
    let mut got = 0;
    while let Some(msg) = reader.recv().await {
        match msg {
            Msg::Chunk(chunk) => got += chunk.payload.len(),
            other => panic!("{other:?}"),
        }
    }
    assert!(got < STREAMS * window as usize, "{got} bytes, not closed");
    assert_eq!(
        reader.close.as_ref().map(|frame| u16::from(frame.code)),
        Some(tilr_bus::close::TOO_SLOW)
    );
}

#[tokio::test]
async fn a_chunk_too_large_to_relay_ends_its_stream() {
    use prost::Message as _;
    let server = start();
    let mut provider = server
        .raw_greeted(
            "huge",
            vec![MethodSpec::stream("test.huge", Lane::Interactive)],
        )
        .await;
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    // The reader's id takes ten bytes, the provider's hop one or two: a chunk
    // at the frame limit with the hop is over it with the reader's id.
    let id = u64::MAX;
    reader
        .send(call_with(
            id,
            "test.huge",
            "",
            tilr_bus::limits::MAX_STREAM_WINDOW,
            0,
        ))
        .await;
    let (hop, _, _) = next_call(&mut provider).await;
    let envelope = |len: usize| Envelope {
        msg: Some(chunk(hop, len)),
    };
    let mut len = tilr_bus::codec::MAX_FRAME;
    while envelope(len).encoded_len() > tilr_bus::codec::MAX_FRAME {
        len -= 1;
    }
    provider.send(chunk(hop, len)).await;
    let (got, code, message) = error_of(&reader.recv().await.unwrap()).expect("an error");
    assert_eq!((got, code.as_str()), (id, codes::TOO_LARGE), "{message}");
    cancelled(&mut provider, hop).await;
    // The stream is over: nothing more of it.
    reader.send(call(1, "bus.ping", "")).await;
    let next = reader.recv().await.unwrap();
    assert!(
        matches!(&next, Msg::Reply(reply) if reply.id == 1),
        "{next:?}"
    );
}

#[tokio::test]
async fn a_reader_that_never_credits_cannot_grow_the_relay() {
    let server = start();
    let mut provider = server
        .raw_greeted(
            "relay",
            vec![
                MethodSpec::stream("test.relay", Lane::Bulk),
                MethodSpec::unary("test.marker"),
            ],
        )
        .await;
    let mut reader = server.raw_greeted("reader", Vec::new()).await;
    // Streams opened one after another, each sent whole and ended, never read:
    // what the broker holds of them stops new ones (busy). A marker call after
    // each tells the provider whether the stream came: calls from one
    // connection to one provider keep their order.
    const MAX: u64 = 64;
    let mut opened = 0;
    for i in 1..=MAX {
        reader.send(call(i, "test.relay", "")).await;
        reader.send(call(1000 + i, "test.marker", "")).await;
        let (hop, method, window) = next_call(&mut provider).await;
        if method == "test.marker" {
            break;
        }
        opened += 1;
        assert_eq!(
            send_within_credit(&mut provider, hop, window, window, 64 << 10).await,
            window
        );
        provider
            .send(Msg::End(End {
                id: hop,
                payload: Bytes::new(),
            }))
            .await;
        let (_, method, _) = next_call(&mut provider).await;
        assert_eq!(method, "test.marker");
    }
    assert!(
        opened < MAX,
        "{opened} streams of 1 MiB held for a reader that reads none"
    );
    let budget = (16 << 20) / MIB as u64;
    assert!(opened >= budget, "busy after {opened}");
    // It reads at last: every stream opened, whole, then the busy one.
    let (mut ends, mut busy) = (0, false);
    while ends < opened || !busy {
        match reader.recv().await.expect("open") {
            Msg::Chunk(_) => {}
            Msg::End(_) => ends += 1,
            msg => {
                let (id, code, _) = error_of(&msg).expect("an error");
                assert_eq!((id, code.as_str()), (opened + 1, codes::BUSY));
                busy = true;
            }
        }
    }
}
