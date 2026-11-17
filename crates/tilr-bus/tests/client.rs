//! The Rust client against a partner in the test that plays the broker.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use common::{Peer, WAIT, ack};
use prost::Message;
use tilr_bus::bus::{
    self, Call, Cancel, Chunk, ConnectionError, CredentialUpdate, Credit, End, Error, Event, Lane,
    MethodSpec, Reply, Select, envelope::Msg, reply,
};
use tilr_bus::{
    Bus, BusError, CallOptions, ConnectOptions, Credential, Handler, StreamOptions, Transport,
    codes, pane,
};
use tokio::net::TcpListener;

async fn connected(options: ConnectOptions) -> (Bus, Peer) {
    let (transport, mut peer) = Peer::pair();
    let (bus, _) = tokio::join!(Bus::connect(transport, options), async {
        peer.greet(ack("owner")).await
    });
    (bus.unwrap(), peer)
}

fn ok(id: u64, payload: &'static [u8]) -> Msg {
    Msg::Reply(Reply {
        id,
        result: Some(reply::Result::Payload(Bytes::from_static(payload))),
        content_type: String::new(),
    })
}

fn err(id: u64, code: &str, message: &str) -> Msg {
    Msg::Reply(Reply {
        id,
        result: Some(reply::Result::Error(Error {
            code: code.into(),
            message: message.into(),
        })),
        content_type: String::new(),
    })
}

async fn expect_call(peer: &mut Peer) -> Call {
    match peer.recv().await {
        Msg::Call(call) => call,
        other => panic!("expected a call, got {other:?}"),
    }
}

#[tokio::test]
async fn hello_comes_first_and_the_identity_is_the_brokers_answer() {
    let (transport, mut peer) = Peer::pair();
    let options = ConnectOptions::new("tester")
        .client_id("6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4")
        .provide(
            MethodSpec::unary("ui.notify").with_select(Select::LastActive),
            Handler::unary(|_| async { Ok("ok") }),
        );
    let mut instance_ack = ack("instance:t-1");
    instance_ack.credential = Bytes::from_static(b"minted");
    let (bus, hello) = tokio::join!(Bus::connect(transport, options), peer.greet(instance_ack));
    let bus = bus.unwrap();
    assert_eq!(hello.protocol_version, 1);
    assert_eq!(hello.client_name, "tester");
    assert_eq!(hello.client_id, "6f1c2a9e-3b7d-4c1f-9a52-0e8d7b61c3f4");
    assert_eq!(hello.provides.len(), 1);
    assert_eq!(hello.provides[0].method, "ui.notify");
    assert_eq!(hello.provides[0].select(), Select::LastActive);
    assert_eq!(hello.provides[0].kind(), bus::Kind::Unary);
    assert_eq!(bus.identity(), "instance:t-1");
    assert_eq!(bus.connection_id(), 7);
    assert_eq!(bus.seq_base(), 7 << 32);
    assert_eq!(bus.hello_ack().server_id, "server-1");
    assert_eq!(
        bus.credential(),
        Some(Credential::from_bytes(&b"minted"[..]))
    );
}

#[tokio::test]
async fn a_refused_hello_is_an_error_with_the_brokers_code() {
    let (transport, mut peer) = Peer::pair();
    let (result, _) = tokio::join!(Bus::connect(transport, ConnectOptions::new("old")), async {
        let Msg::Hello(_) = peer.recv().await else {
            panic!("no hello");
        };
        peer.send(Msg::ConnectionError(ConnectionError {
            code: codes::UNSUPPORTED_VERSION.into(),
            message: "unsupported protocol version 1: this server speaks 2".into(),
        }))
        .await;
    });
    let error = result.unwrap_err();
    assert_eq!(error.code, codes::UNSUPPORTED_VERSION);
    assert!(error.message.contains("this server speaks 2"), "{error}");
}

#[tokio::test]
async fn a_call_is_answered_with_its_reply() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let credential = Credential::from_bytes(&b"bearer"[..]);
    let options = CallOptions::default()
        .target("instance:t-2")
        .timeout(Duration::from_secs(5))
        .content_type("application/octet-stream")
        .credential(credential)
        .trace_id("trace-1");
    let (reply, ()) = tokio::join!(
        bus.call("test.echo", &b"\x00\xff\xfe"[..], options),
        async {
            let call = expect_call(&mut peer).await;
            assert_eq!(call.method, "test.echo");
            assert_eq!(call.target, "instance:t-2");
            assert_eq!(call.payload.as_ref(), b"\x00\xff\xfe");
            assert_eq!(call.content_type, "application/octet-stream");
            assert_eq!(call.credential.as_ref(), b"bearer");
            assert_eq!(call.trace_id, "trace-1");
            assert_eq!(call.timeout_ms, 5000);
            assert_eq!(call.window, 0);
            peer.send(Msg::Reply(Reply {
                id: call.id,
                result: Some(reply::Result::Payload(call.payload)),
                content_type: call.content_type,
            }))
            .await;
        }
    );
    let reply = reply.unwrap();
    assert_eq!(reply.payload.as_ref(), b"\x00\xff\xfe");
    assert_eq!(reply.content_type, "application/octet-stream");
}

#[tokio::test]
async fn a_failed_call_says_why_with_the_code_and_message() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let (reply, ()) = tokio::join!(bus.call("nobody.here", "", CallOptions::default()), async {
        let call = expect_call(&mut peer).await;
        peer.send(err(call.id, codes::NOT_FOUND, "no provider of nobody.here"))
            .await;
    });
    assert_eq!(
        reply.unwrap_err(),
        BusError::new(codes::NOT_FOUND, "no provider of nobody.here")
    );
}

#[tokio::test]
async fn a_typed_call_sends_and_reads_protobuf() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let request = pane::CreateRequest {
        cols: 100,
        rows: 30,
        profile: "demo".into(),
        ..Default::default()
    };
    let (reply, ()) = tokio::join!(
        bus.call_typed::<_, pane::PaneInfo>("pane.create", &request, CallOptions::default()),
        async {
            let call = expect_call(&mut peer).await;
            let got = pane::CreateRequest::decode(call.payload).unwrap();
            assert_eq!(got.profile, "demo");
            let info = pane::PaneInfo {
                pane_id: "p-1".into(),
                cols: got.cols,
                rows: got.rows,
                created_by: "owner".into(),
                profile: got.profile,
                ..Default::default()
            };
            peer.send(Msg::Reply(Reply {
                id: call.id,
                result: Some(reply::Result::Payload(info.encode_to_vec().into())),
                content_type: String::new(),
            }))
            .await;
        }
    );
    let info = reply.unwrap();
    assert_eq!((info.cols, info.rows), (100, 30));
    assert_eq!(info.created_by, "owner");

    // A reply that is not the message is bad_reply.
    let (reply, ()) = tokio::join!(
        bus.call_typed::<_, pane::PaneInfo>("pane.create", &request, CallOptions::default()),
        async {
            let call = expect_call(&mut peer).await;
            peer.send(ok(call.id, b"\xff\xff\xff")).await;
        }
    );
    assert_eq!(reply.unwrap_err().code, codes::BAD_REPLY);
}

#[tokio::test]
async fn a_call_that_is_given_up_is_cancelled() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let options = CallOptions::default().timeout(Duration::from_millis(100));
    let (reply, ()) = tokio::join!(bus.call("test.sleep", "", options), async {
        let call = expect_call(&mut peer).await;
        let Msg::Cancel(cancel) = peer.recv().await else {
            panic!("no cancel");
        };
        assert_eq!(cancel.id, call.id);
        // A late reply is dropped.
        peer.send(ok(call.id, b"late")).await;
    });
    assert_eq!(reply.unwrap_err().code, codes::TIMEOUT);

    // Dropping the future of a call cancels it too.
    let call = tokio::spawn({
        let bus = bus.clone();
        async move { bus.call("test.sleep", "", CallOptions::default()).await }
    });
    let sent = expect_call(&mut peer).await;
    call.abort();
    let Msg::Cancel(cancel) = peer.recv().await else {
        panic!("no cancel");
    };
    assert_eq!(cancel.id, sent.id);

    // The connection still works.
    let (reply, ()) = tokio::join!(bus.call("test.echo", "x", CallOptions::default()), async {
        let call = expect_call(&mut peer).await;
        peer.send(ok(call.id, b"x")).await;
    });
    assert_eq!(reply.unwrap().payload.as_ref(), b"x");
}

#[tokio::test]
async fn the_end_of_the_connection_fails_pending_calls_unavailable() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let (reply, ()) = tokio::join!(bus.call("test.sleep", "", CallOptions::default()), async {
        expect_call(&mut peer).await;
        drop(peer);
    });
    assert_eq!(reply.unwrap_err().code, codes::UNAVAILABLE);
    let info = tokio::time::timeout(WAIT, bus.closed()).await.unwrap();
    assert_eq!(info.code, None);
    assert!(bus.is_closed());
    let error = bus
        .call("test.echo", "", CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::UNAVAILABLE);
}

#[tokio::test]
async fn a_connection_error_reaches_pending_calls_with_its_code() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let (reply, ()) = tokio::join!(bus.call("test.sleep", "", CallOptions::default()), async {
        expect_call(&mut peer).await;
        peer.send(Msg::ConnectionError(ConnectionError {
            code: codes::DENIED.into(),
            message: "credential revoked".into(),
        }))
        .await;
    });
    assert_eq!(
        reply.unwrap_err(),
        BusError::new(codes::DENIED, "credential revoked")
    );
}

#[tokio::test]
async fn calls_wait_for_a_slot_beyond_max_in_flight() {
    let (transport, mut peer) = Peer::pair();
    let mut small = ack("owner");
    small.max_in_flight = 2;
    let (bus, _) = tokio::join!(
        Bus::connect(transport, ConnectOptions::new("t")),
        peer.greet(small)
    );
    let bus = bus.unwrap();
    let calls: Vec<_> = (0..3)
        .map(|_| {
            let bus = bus.clone();
            tokio::spawn(async move { bus.call("test.sleep", "", CallOptions::default()).await })
        })
        .collect();
    let first = expect_call(&mut peer).await;
    let second = expect_call(&mut peer).await;
    assert!(
        peer.quiet(Duration::from_millis(200)).await,
        "a third call went out"
    );
    peer.send(ok(first.id, b"1")).await;
    let third = expect_call(&mut peer).await;
    peer.send(ok(second.id, b"2")).await;
    peer.send(ok(third.id, b"3")).await;
    for call in calls {
        call.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn a_payload_over_the_limit_is_refused_without_sending() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let payload = vec![0u8; tilr_bus::codec::MAX_FRAME + 1];
    let error = bus
        .call("test.echo", payload, CallOptions::default())
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::TOO_LARGE);
    assert!(peer.quiet(Duration::from_millis(100)).await);
}

#[tokio::test]
async fn a_stream_is_read_chunk_by_chunk_and_credit_goes_back() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let options = StreamOptions::default().window(10);
    let mut stream = bus.stream("test.count", "3", options).await.unwrap();
    let call = expect_call(&mut peer).await;
    assert_eq!((call.method.as_str(), call.window), ("test.count", 10));
    for n in [b"1", b"2", b"3"] {
        peer.send(Msg::Chunk(Chunk {
            id: call.id,
            payload: Bytes::from_static(n),
            content_encoding: String::new(),
        }))
        .await;
    }
    peer.send(Msg::End(End {
        id: call.id,
        payload: Bytes::from_static(b"done"),
    }))
    .await;
    let mut got = Vec::new();
    while let Some(chunk) = stream.next().await {
        got.push(chunk.unwrap().payload);
    }
    assert_eq!(got, ["1", "2", "3"]);
    assert_eq!(stream.end_payload().map(|b| b.as_ref()), Some(&b"done"[..]));
    for _ in 0..3 {
        let Msg::Credit(credit) = peer.recv().await else {
            panic!("no credit");
        };
        assert_eq!((credit.id, credit.bytes), (call.id, 1));
    }
}

#[tokio::test]
async fn a_stream_answered_with_an_error_ends_with_it() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let mut stream = bus
        .stream("fs.read", "", StreamOptions::default())
        .await
        .unwrap();
    let call = expect_call(&mut peer).await;
    assert_eq!(call.window, tilr_bus::limits::STREAM_WINDOW);
    peer.send(err(call.id, codes::NOT_FOUND, "no such file"))
        .await;
    assert_eq!(
        stream.next().await.unwrap().unwrap_err().code,
        codes::NOT_FOUND
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_provider_that_overruns_the_credit_ends_the_stream_as_bad_reply() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let mut stream = bus
        .stream("test.flood", "", StreamOptions::default().window(4))
        .await
        .unwrap();
    let call = expect_call(&mut peer).await;
    peer.send(Msg::Chunk(Chunk {
        id: call.id,
        payload: Bytes::from_static(b"12345"),
        content_encoding: String::new(),
    }))
    .await;
    assert_eq!(
        stream.next().await.unwrap().unwrap_err().code,
        codes::BAD_REPLY
    );
    let Msg::Cancel(cancel) = peer.recv().await else {
        panic!("no cancel");
    };
    assert_eq!(cancel.id, call.id);
}

#[tokio::test]
async fn leaving_a_stream_early_cancels_it() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let mut stream = bus
        .stream("test.count", "1000000", StreamOptions::default())
        .await
        .unwrap();
    let call = expect_call(&mut peer).await;
    peer.send(Msg::Chunk(Chunk {
        id: call.id,
        payload: Bytes::from_static(b"1"),
        content_encoding: String::new(),
    }))
    .await;
    stream.next().await.unwrap().unwrap();
    drop(stream);
    let Msg::Credit(_) = peer.recv().await else {
        panic!("no credit");
    };
    let Msg::Cancel(cancel) = peer.recv().await else {
        panic!("no cancel");
    };
    assert_eq!(cancel.id, call.id);
}

fn provider(options: ConnectOptions) -> ConnectOptions {
    options
        .provide(
            MethodSpec::unary("test.echo"),
            Handler::unary(|request| async move {
                Ok(tilr_bus::Reply::new(request.payload).with_content_type(request.content_type))
            }),
        )
        .provide(
            MethodSpec::unary("test.from"),
            Handler::unary(|request| async move {
                Ok(format!(
                    "{} {} {}",
                    request.from,
                    request.timeout_ms,
                    request.credential.len()
                ))
            }),
        )
        .provide(
            MethodSpec::unary("test.fail"),
            Handler::unary(|_| async { Err::<Bytes, _>(BusError::bad_request("x")) }),
        )
        .provide(
            MethodSpec::unary("test.crash"),
            Handler::unary(|_| async { panic!("boom") as Result<Bytes, BusError> }),
        )
        .provide(
            MethodSpec::stream("test.count", Lane::Interactive),
            Handler::stream(|request, out| async move {
                let n: u32 = String::from_utf8_lossy(&request.payload).parse().unwrap();
                for i in 1..=n {
                    out.write(format!("{i:02}")).await?;
                }
                Ok(Some(Bytes::from_static(b"end")))
            }),
        )
}

#[tokio::test]
async fn a_provided_method_answers_and_sees_who_called() {
    let (_bus, mut peer) = connected(provider(ConnectOptions::new("t"))).await;
    peer.send(Msg::Call(Call {
        id: 1,
        method: "test.echo".into(),
        payload: Bytes::from_static(b"\x00\xff"),
        content_type: "application/octet-stream".into(),
        ..Default::default()
    }))
    .await;
    let Msg::Reply(reply) = peer.recv().await else {
        panic!("no reply");
    };
    assert_eq!(reply.id, 1);
    assert_eq!(reply.content_type, "application/octet-stream");
    assert_eq!(
        reply.result,
        Some(reply::Result::Payload(Bytes::from_static(b"\x00\xff")))
    );

    peer.send(Msg::Call(Call {
        id: 2,
        method: "test.from".into(),
        from: "instance:t-1".into(),
        timeout_ms: 600,
        credential: Bytes::from_static(b"abc"),
        ..Default::default()
    }))
    .await;
    let Msg::Reply(reply) = peer.recv().await else {
        panic!("no reply");
    };
    assert_eq!(
        reply.result,
        Some(reply::Result::Payload(Bytes::from_static(
            b"instance:t-1 600 3"
        )))
    );
}

#[tokio::test]
async fn a_method_not_provided_is_answered_not_supported() {
    let (_bus, mut peer) = connected(provider(ConnectOptions::new("t"))).await;
    peer.send(Msg::Call(Call {
        id: 5,
        method: "nobody.here".into(),
        ..Default::default()
    }))
    .await;
    let Msg::Reply(Reply {
        id: 5,
        result: Some(reply::Result::Error(error)),
        ..
    }) = peer.recv().await
    else {
        panic!("no error reply");
    };
    assert_eq!(error.code, codes::NOT_SUPPORTED);
    assert!(error.message.contains("nobody.here"), "{error:?}");
}

#[tokio::test]
async fn a_handlers_error_keeps_its_code_and_a_panic_is_a_handler_error() {
    let (_bus, mut peer) = connected(provider(ConnectOptions::new("t"))).await;
    for (id, method) in [(1, "test.fail"), (2, "test.crash")] {
        peer.send(Msg::Call(Call {
            id,
            method: method.into(),
            ..Default::default()
        }))
        .await;
    }
    let mut errors = Vec::new();
    for _ in 0..2 {
        let Msg::Reply(Reply {
            id,
            result: Some(reply::Result::Error(error)),
            ..
        }) = peer.recv().await
        else {
            panic!("no error reply");
        };
        errors.push((id, error.code, error.message));
    }
    errors.sort();
    assert_eq!(errors[0], (1, codes::BAD_REQUEST.into(), "x".into()));
    assert_eq!(errors[1].1, codes::HANDLER_ERROR);
    assert!(errors[1].2.contains("boom"), "{errors:?}");
}

#[tokio::test]
async fn a_cancelled_handler_stops_without_a_reply() {
    let started = Arc::new(tokio::sync::Notify::new());
    let finished = Arc::new(AtomicBool::new(false));
    let options = ConnectOptions::new("t").provide(
        MethodSpec::unary("test.sleep"),
        Handler::unary({
            let started = started.clone();
            let finished = finished.clone();
            move |_| {
                let started = started.clone();
                let finished = finished.clone();
                async move {
                    started.notify_one();
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    finished.store(true, Ordering::SeqCst);
                    Ok("slept")
                }
            }
        }),
    );
    let (_bus, mut peer) = connected(options).await;
    peer.send(Msg::Call(Call {
        id: 3,
        method: "test.sleep".into(),
        ..Default::default()
    }))
    .await;
    started.notified().await;
    peer.send(Msg::Cancel(Cancel {
        id: 3,
        reason: "timeout".into(),
    }))
    .await;
    assert!(
        peer.quiet(Duration::from_millis(1200)).await,
        "a reply came"
    );
    assert!(!finished.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_provided_stream_is_paced_by_the_readers_credit() {
    let (_bus, mut peer) = connected(provider(ConnectOptions::new("t"))).await;
    peer.send(Msg::Call(Call {
        id: 9,
        method: "test.count".into(),
        payload: Bytes::from_static(b"3"),
        window: 4,
        ..Default::default()
    }))
    .await;
    // A window of 4 bytes lets two chunks of 2 through.
    for expected in [b"01", b"02"] {
        let Msg::Chunk(chunk) = peer.recv().await else {
            panic!("no chunk");
        };
        assert_eq!((chunk.id, chunk.payload.as_ref()), (9, &expected[..]));
    }
    assert!(
        peer.quiet(Duration::from_millis(200)).await,
        "sent past the credit"
    );
    peer.send(Msg::Credit(Credit { id: 9, bytes: 2 })).await;
    let Msg::Chunk(chunk) = peer.recv().await else {
        panic!("no chunk");
    };
    assert_eq!(chunk.payload.as_ref(), b"03");
    let Msg::End(end) = peer.recv().await else {
        panic!("no end");
    };
    assert_eq!((end.id, end.payload.as_ref()), (9, &b"end"[..]));
}

#[tokio::test]
async fn events_reach_the_subscriptions_whose_pattern_matches() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let (one, ()) = tokio::join!(bus.subscribe("pane.*"), async {
        let Msg::Subscribe(subscribe) = peer.recv().await else {
            panic!("no subscribe");
        };
        assert_eq!(subscribe.pattern, "pane.*");
        peer.send(Msg::Reply(Reply {
            id: subscribe.id,
            result: None,
            content_type: String::new(),
        }))
        .await;
    });
    let mut one = one.unwrap();
    let (all, ()) = tokio::join!(bus.subscribe("pane.>"), async {
        let Msg::Subscribe(subscribe) = peer.recv().await else {
            panic!("no subscribe");
        };
        peer.send(Msg::Reply(Reply {
            id: subscribe.id,
            result: None,
            content_type: String::new(),
        }))
        .await;
    });
    let mut all = all.unwrap();
    for subject in ["pane.created", "pane.x.y", "fs.changed.1"] {
        peer.send(Msg::Event(Event {
            subject: subject.into(),
            payload: Bytes::from_static(b"p"),
            from: "tilrd".into(),
            scope: "pane:1".into(),
            ..Default::default()
        }))
        .await;
    }
    let first = one.next().await.unwrap();
    assert_eq!(
        (
            first.subject.as_str(),
            first.from.as_str(),
            first.scope.as_str()
        ),
        ("pane.created", "tilrd", "pane:1")
    );
    assert_eq!(all.next().await.unwrap().subject, "pane.created");
    assert_eq!(all.next().await.unwrap().subject, "pane.x.y");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), one.next())
            .await
            .is_err(),
        "pane.* got pane.x.y or fs.changed.1"
    );

    let id = {
        drop(one);
        let Msg::Unsubscribe(unsubscribe) = peer.recv().await else {
            panic!("no unsubscribe");
        };
        unsubscribe.id
    };
    assert!(id > 0);
}

#[tokio::test]
async fn a_denied_subscription_fails_with_denied() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let (result, ()) = tokio::join!(bus.subscribe("pane.*"), async {
        let Msg::Subscribe(subscribe) = peer.recv().await else {
            panic!("no subscribe");
        };
        peer.send(err(subscribe.id, codes::DENIED, "subscribe pane.*"))
            .await;
    });
    assert_eq!(result.err().unwrap().code, codes::DENIED);
    assert_eq!(
        bus.subscribe("pane").await.err().unwrap().code,
        codes::BAD_REQUEST
    );
}

#[tokio::test]
async fn emitting_activating_and_renewing_send_their_envelopes() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    bus.emit("test.ping", "1").await.unwrap();
    let Msg::Event(event) = peer.recv().await else {
        panic!("no event");
    };
    assert_eq!(
        (event.subject.as_str(), event.payload.as_ref()),
        ("test.ping", &b"1"[..])
    );
    assert_eq!(
        bus.emit("Not a subject", "").await.unwrap_err().code,
        codes::BAD_REQUEST
    );

    bus.activate().await.unwrap();
    let Msg::Activate(_) = peer.recv().await else {
        panic!("no activate");
    };

    let fresh = Credential::from_bytes(&b"fresh"[..]);
    bus.update_credential(fresh.clone()).await.unwrap();
    let Msg::CredentialUpdate(update) = peer.recv().await else {
        panic!("no credential update");
    };
    assert_eq!(update.credential.as_ref(), b"fresh");
    assert_eq!(bus.credential(), Some(fresh));

    // A renewal from the broker replaces the token too.
    peer.send(Msg::CredentialUpdate(CredentialUpdate {
        credential: Bytes::from_static(b"renewed"),
    }))
    .await;
    let renewed = Some(Credential::from_bytes(&b"renewed"[..]));
    for _ in 0..100 {
        if bus.credential() == renewed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(bus.credential(), renewed);
}

#[tokio::test]
async fn the_same_call_works_over_fd_socket_and_websocket() {
    // Instance socket: Hello carries the instance token.
    let dir = std::env::temp_dir().join(format!("tilr-bus-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sock");
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let transport = Transport::Socket {
        path: path.clone(),
        token: "00ff".into(),
    };
    let (bus, hello) = tokio::join!(
        Bus::connect(transport, ConnectOptions::new("sock")),
        async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut peer = Peer::from_unix(stream);
            let hello = peer.greet(ack("instance:t-1")).await;
            (peer, hello)
        }
    );
    let (mut peer, hello) = hello;
    assert_eq!(hello.instance_token, "00ff");
    echo_round_trip(&bus.unwrap(), &mut peer).await;
    let _ = std::fs::remove_dir_all(&dir);

    // WebSocket on TCP: subprotocol, access token, credential and origin.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = Transport::WebSocket {
        url: format!("ws://{}/", listener.local_addr().unwrap()),
        access_token: Some("secret".into()),
        credential: Some(Credential::from_bytes(&b"tok"[..])),
        origin: Some("tilr://app".into()),
    };
    let (bus, peer) = tokio::join!(Bus::connect(transport, ConnectOptions::new("ws")), async {
        let mut peer = Peer::accept_ws(&listener).await;
        let hello = peer.greet(ack("token:web")).await;
        assert_eq!(hello.access_token, "secret");
        assert_eq!(hello.credential.as_ref(), b"tok");
        peer
    });
    let (bus, mut peer) = (bus.unwrap(), peer);
    assert_eq!(peer.header("sec-websocket-protocol"), Some("tilr.bus.v1"));
    assert_eq!(peer.header("origin"), Some("tilr://app"));
    echo_round_trip(&bus, &mut peer).await;

    // Large envelopes are compressed and come back whole.
    let big = Bytes::from("0123456789".repeat(100_000));
    let (reply, ()) = tokio::join!(
        bus.call("test.echo", big.clone(), CallOptions::default()),
        async {
            let call = expect_call(&mut peer).await;
            peer.send(Msg::Reply(Reply {
                id: call.id,
                result: Some(reply::Result::Payload(call.payload)),
                content_type: String::new(),
            }))
            .await;
        }
    );
    assert_eq!(reply.unwrap().payload, big);
}

async fn echo_round_trip(bus: &Bus, peer: &mut Peer) {
    let (reply, ()) = tokio::join!(bus.call("test.echo", "x", CallOptions::default()), async {
        let call = expect_call(peer).await;
        peer.send(Msg::Reply(Reply {
            id: call.id,
            result: Some(reply::Result::Payload(call.payload)),
            content_type: String::new(),
        }))
        .await;
    });
    assert_eq!(reply.unwrap().payload.as_ref(), b"x");
}

#[tokio::test]
async fn a_bad_websocket_message_is_skipped_and_a_close_code_is_reported() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = Transport::WebSocket {
        url: format!("ws://{}/", listener.local_addr().unwrap()),
        access_token: None,
        credential: None,
        origin: None,
    };
    let (bus, mut peer) = tokio::join!(Bus::connect(transport, ConnectOptions::new("ws")), async {
        let mut peer = Peer::accept_ws(&listener).await;
        peer.greet(ack("owner")).await;
        peer
    });
    let bus = bus.unwrap();
    peer.send_raw_ws(tokio_tungstenite::tungstenite::Message::Binary(
        Bytes::from_static(&[9, 1, 2]),
    ))
    .await;
    peer.send(Msg::ConnectionError(ConnectionError {
        code: codes::BAD_FRAME.into(),
        message: "bad packet: unknown packet flag 9".into(),
    }))
    .await;
    echo_round_trip(&bus, &mut peer).await;
    peer.close_ws(tilr_bus::close::SUPERSEDED, "superseded")
        .await;
    let info = tokio::time::timeout(WAIT, bus.closed()).await.unwrap();
    assert_eq!(info.code, Some(4000));
    assert_eq!(info.reason, "superseded");
}

#[tokio::test]
async fn dropping_the_last_handle_closes_the_connection() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    drop(bus);
    assert!(peer.try_recv().await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_that_comes_with_hello_ack_is_answered() {
    let (transport, mut peer) = Peer::pair();
    let options = provider(ConnectOptions::new("t"));
    let (bus, ()) = tokio::join!(Bus::connect(transport, options), async {
        let Msg::Hello(_) = peer.recv().await else {
            panic!("the first envelope is not Hello");
        };
        // HelloAck and a Call in one write: the client reads both at once.
        peer.send_together(vec![
            Msg::HelloAck(ack("owner")),
            Msg::Call(Call {
                id: 1,
                method: "test.echo".into(),
                payload: Bytes::from_static(b"early"),
                ..Default::default()
            }),
        ])
        .await;
    });
    let _bus = bus.unwrap();
    let Msg::Reply(reply) = peer.recv().await else {
        panic!("no reply");
    };
    assert_eq!(reply.id, 1);
    assert_eq!(
        reply.result,
        Some(reply::Result::Payload(Bytes::from_static(b"early")))
    );
}

#[tokio::test]
async fn a_handler_that_panics_before_its_future_is_a_handler_error() {
    type Ready = std::future::Ready<Result<Bytes, BusError>>;
    type ReadyEnd = std::future::Ready<Result<Option<Bytes>, BusError>>;
    let options = ConnectOptions::new("t")
        .provide(
            MethodSpec::unary("test.early_panic"),
            Handler::unary(|_| -> Ready { panic!("before the future") }),
        )
        .provide(
            MethodSpec::stream("test.early_panic_stream", Lane::Interactive),
            Handler::stream(|_, _| -> ReadyEnd { panic!("before the stream") }),
        );
    let (_bus, mut peer) = connected(options).await;
    for (id, method, message) in [
        (1, "test.early_panic", "before the future"),
        (2, "test.early_panic_stream", "before the stream"),
        // The connection and the method still work after one.
        (3, "test.early_panic", "before the future"),
    ] {
        peer.send(Msg::Call(Call {
            id,
            method: method.into(),
            ..Default::default()
        }))
        .await;
        let Msg::Reply(Reply {
            id: got,
            result: Some(reply::Result::Error(error)),
            ..
        }) = peer.recv().await
        else {
            panic!("no error reply");
        };
        assert_eq!(got, id);
        assert_eq!(error.code, codes::HANDLER_ERROR);
        assert!(error.message.contains(message), "{error:?}");
    }
    // A cancel of a call that was answered finds nothing to stop.
    peer.send(Msg::Cancel(Cancel {
        id: 1,
        reason: "late".into(),
    }))
    .await;
    assert!(peer.quiet(Duration::from_millis(100)).await);
}

#[tokio::test]
async fn a_server_that_accepts_and_says_nothing_times_out_the_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let transport = Transport::WebSocket {
        url: format!("ws://{}/", listener.local_addr().unwrap()),
        access_token: None,
        credential: None,
        origin: None,
    };
    let silent = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(socket);
    });
    let options = ConnectOptions::new("t").handshake_timeout(Duration::from_millis(300));
    let started = std::time::Instant::now();
    let error = Bus::connect(transport, options).await.unwrap_err();
    assert_eq!(error.code, codes::TIMEOUT, "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    silent.abort();
}

#[tokio::test]
async fn a_stream_that_ended_frees_its_slot() {
    let (transport, mut peer) = Peer::pair();
    let mut one = ack("owner");
    one.max_in_flight = 1;
    let (bus, _) = tokio::join!(
        Bus::connect(transport, ConnectOptions::new("t")),
        peer.greet(one)
    );
    let bus = bus.unwrap();

    // Ended with End, still held by the caller.
    let mut stream = bus
        .stream("test.count", "0", StreamOptions::default())
        .await
        .unwrap();
    let call = expect_call(&mut peer).await;
    peer.send(Msg::End(End {
        id: call.id,
        payload: Bytes::new(),
    }))
    .await;
    assert!(stream.next().await.is_none());
    let (reply, ()) = tokio::join!(bus.call("test.echo", "x", CallOptions::default()), async {
        let call = expect_call(&mut peer).await;
        peer.send(ok(call.id, b"x")).await;
    });
    reply.unwrap();

    // Ended with an error.
    let mut failed = bus
        .stream("test.count", "0", StreamOptions::default())
        .await
        .unwrap();
    let call = expect_call(&mut peer).await;
    peer.send(err(call.id, codes::NOT_FOUND, "gone")).await;
    assert_eq!(
        failed.next().await.unwrap().unwrap_err().code,
        codes::NOT_FOUND
    );
    let (reply, ()) = tokio::join!(bus.call("test.echo", "y", CallOptions::default()), async {
        let call = expect_call(&mut peer).await;
        peer.send(ok(call.id, b"y")).await;
    });
    reply.unwrap();
    drop((stream, failed));
}

#[tokio::test]
async fn a_full_subscription_counts_the_events_it_dropped() {
    let (bus, mut peer) = connected(ConnectOptions::new("t")).await;
    let (subscription, ()) = tokio::join!(bus.subscribe("test.*"), async {
        let Msg::Subscribe(subscribe) = peer.recv().await else {
            panic!("no subscribe");
        };
        peer.send(Msg::Reply(Reply {
            id: subscribe.id,
            result: None,
            content_type: String::new(),
        }))
        .await;
    });
    let mut subscription = subscription.unwrap();
    assert_eq!(subscription.dropped(), 0);
    for n in 0..1030 {
        peer.send(Msg::Event(Event {
            subject: "test.tick".into(),
            payload: Bytes::from(n.to_string()),
            ..Default::default()
        }))
        .await;
    }
    // A reply after them: once it is in, every event before it was handled.
    let (reply, ()) = tokio::join!(bus.call("test.echo", "", CallOptions::default()), async {
        let call = expect_call(&mut peer).await;
        peer.send(ok(call.id, b"")).await;
    });
    reply.unwrap();
    assert_eq!(subscription.dropped(), 6);
    let mut kept = 0;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(50), subscription.next()).await
    {
        assert_eq!(event.payload, kept.to_string());
        kept += 1;
    }
    assert_eq!(kept, 1024);
}
