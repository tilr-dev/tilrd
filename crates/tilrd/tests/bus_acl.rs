//! Use cases: rights on the bus (change `bus-rights`, design R1-R8). The owner mints
//! tokens with `bus.token` during each test; outside participants are `tilr_bus`
//! clients and the test runtime on the Unix socket with those tokens (identity
//! `token:<name>`). Every key and token is made by the server under test.

mod common;

use std::os::unix::fs::PermissionsExt;

use bytes::Bytes;
use common::TIMEOUT;
use common::bus::{BusOptions, BusServer, text};
#[cfg(feature = "failpoints")]
use common::bus_rights::{HOUR_US, TestClock};
use common::bus_rights::{
    Rights, audit_lines, calls, connect, mint, participant, restart, runtime, state_dir, try_mint,
    wait_logged,
};
use tilr_bus::bus::MethodSpec;
use tilr_bus::proto::tilr::bus::v1 as wire;
use tilr_bus::{
    Bus, BusError, CallOptions, ConnectOptions, Credential, Handler, Request, Transport, codes,
};

fn start() -> BusServer {
    BusServer::start(BusOptions::default())
}

/// [`start`] for a test that runs the test runtime: built before any connection
/// opens, since the build blocks this test's runtime and an owner left silent that
/// long would be dropped.
fn start_for_runtime() -> BusServer {
    common::bus::test_runtime_path();
    start()
}

fn with(credential: &Credential) -> CallOptions {
    CallOptions::default().credential(credential.clone())
}

fn to(target: &str) -> CallOptions {
    CallOptions::default().target(target)
}

fn denied(result: Result<String, BusError>) -> String {
    match result {
        Err(error) if error.code == codes::DENIED => error.message,
        other => panic!("expected denied, got {other:?}"),
    }
}

/// The owner providing `test.echo` and `other.echo` (the owner's rights do not
/// expire, so it outlives every token).
async fn echo_provider(server: &BusServer, name: &str) -> Bus {
    let echo = || Handler::unary(|request: Request| async move { Ok(request.payload) });
    server
        .owner_with(
            ConnectOptions::new(name)
                .provide(MethodSpec::unary("test.echo"), echo())
                .provide(MethodSpec::unary("other.echo"), echo()),
        )
        .await
}

/// Wait until `bus` is closed; its close code.
async fn close_code(bus: &Bus) -> Option<u16> {
    tokio::time::timeout(TIMEOUT, bus.closed())
        .await
        .expect("the connection was not closed")
        .code
}

#[tokio::test]
async fn the_root_key_is_made_once() {
    let server = start();
    // Greeted once the broker (and its key) is made.
    let owner = server.owner().await;
    let path = state_dir(&server).join("bus-root.key");
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    let first = std::fs::read(&path).unwrap();
    let (token, _) = mint(&owner, "x", Rights::default().call(&["test.*"])).await;
    drop(owner);

    let (server, kept) = restart(server, BusOptions::default());
    let _owner = server.owner().await;
    let path = kept.0.join("state/bus-root.key");
    assert_eq!(mode(&path), 0o600);
    assert_eq!(std::fs::read(&path).unwrap(), first, "the key was replaced");
    // A token of the first start still verifies.
    let _provider = echo_provider(&server, "echo").await;
    let bus = participant(&server, &token, "x").await;
    assert_eq!(
        calls(&bus, "test.echo", "hi", CallOptions::default())
            .await
            .unwrap(),
        "hi"
    );
}

#[tokio::test]
async fn the_owner_holds_every_right() {
    let server = start();
    let owner = server.owner().await;
    assert_eq!(owner.identity(), "owner");
    assert!(owner.hello_ack().credential.is_empty());
    // Control methods: not denied (`bus.instances` is answered once instances exist).
    if let Err(error) = owner
        .call("bus.instances", Bytes::new(), CallOptions::default())
        .await
    {
        assert_ne!(error.code, codes::DENIED, "{error}");
    }
    let (token, reply) = mint(&owner, "t-1", Rights::default()).await;
    assert!(reply.expires_at_ms > 0);
    assert!(!reply.revocation_id.is_empty());
    owner
        .subscribe("pane.*")
        .await
        .expect("the owner subscribes anywhere");
    owner.emit("my.event", "x").await.unwrap();
    // A participant with a token is not the owner, also on the Unix socket.
    let bus = participant(&server, &token, "t-1").await;
    assert_eq!(bus.identity(), "token:t-1");
    assert!(bus.credential().is_some());
}

#[tokio::test]
async fn a_tcp_participant_needs_a_token() {
    let server = start();
    let tcp = |access_token: Option<String>, credential: Option<Credential>| {
        Bus::connect(
            Transport::WebSocket {
                url: server.url(),
                access_token,
                credential,
                origin: None,
            },
            ConnectOptions::new("tcp"),
        )
    };
    let error = tcp(None, None).await.unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
    let owner = tcp(Some(server.token()), None).await.unwrap();
    assert_eq!(owner.identity(), "owner");
    let (token, _) = mint(&owner, "remote", Rights::default().call(&["test.*"])).await;
    let remote = tcp(None, Some(token)).await.unwrap();
    assert_eq!(remote.identity(), "token:remote");
    let garbage = Credential::from_bytes(Bytes::from_static(b"not a token"));
    let error = tcp(None, Some(garbage)).await.unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
}

#[tokio::test]
async fn a_participant_may_do_what_its_token_lists() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    let _other = echo_provider(&server, "other").await;
    let (token, _) = mint(&owner, "t-1", Rights::default().call(&["test.*"])).await;
    let t1 = participant(&server, &token, "t-1").await;
    assert_eq!(
        calls(&t1, "test.echo", "x", to("token:t-2")).await.unwrap(),
        "x"
    );
    let reason = denied(calls(&t1, "other.echo", "x", CallOptions::default()).await);
    assert_eq!(reason, "token:t-1 has no right to call other.echo");
    // Who called is the token's participant.
    assert_eq!(
        calls(&t1, "test.from", "", to("token:t-2")).await.unwrap(),
        "token:t-1"
    );
}

#[tokio::test]
async fn a_token_without_rights_is_allowed_nothing() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    let (token, _) = mint(&owner, "t-1", Rights::default()).await;
    let t1 = participant(&server, &token, "t-1").await;
    denied(calls(&t1, "test.echo", "x", CallOptions::default()).await);
    denied(calls(&t1, "bus.instances", "", CallOptions::default()).await);
    denied(calls(&t1, "bus.token", "", CallOptions::default()).await);
    let error = t1.subscribe("test.*").await.err().expect("denied");
    assert_eq!(error.code, codes::DENIED);
    // Providing anything is refused at Hello, naming the method.
    let error = connect(
        &server,
        &token,
        ConnectOptions::new("t-1b").provide(
            MethodSpec::unary("test.echo"),
            Handler::unary(|request: Request| async move { Ok(request.payload) }),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, codes::DENIED);
    assert!(error.message.contains("test.echo"), "{error}");
    // `bus.ping` asks for no right, only a valid token.
    t1.call("bus.ping", Bytes::new(), CallOptions::default())
        .await
        .expect("bus.ping");
}

#[tokio::test]
async fn providing_emitting_and_subscribing_need_rights() {
    let server = start();
    let owner = server.owner().await;
    let mut events = owner.subscribe("test.*").await.unwrap();

    // Provide: only inside the right.
    let (narrow, _) = mint(&owner, "p", Rights::default().provide(&["ui.*"])).await;
    let error = connect(
        &server,
        &narrow,
        ConnectOptions::new("p").provide(
            MethodSpec::unary("test.echo"),
            Handler::unary(|request: Request| async move { Ok(request.payload) }),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, codes::DENIED);
    assert!(error.message.contains("test.echo"), "{error}");
    connect(
        &server,
        &narrow,
        ConnectOptions::new("p").provide(
            MethodSpec::unary("ui.notify"),
            Handler::unary(|request: Request| async move { Ok(request.payload) }),
        ),
    )
    .await
    .expect("ui.notify is inside ui.*");

    // Subscribe: test.* yes, pane.* no.
    let (token, _) = mint(
        &owner,
        "t-1",
        Rights::default().subscribe(&["test.*"]).emit(&["test.a"]),
    )
    .await;
    let t1 = participant(&server, &token, "t-1").await;
    t1.subscribe("test.*").await.expect("inside the right");
    t1.subscribe("test.x").await.expect("inside the right");
    let error = t1.subscribe("pane.*").await.err().expect("denied");
    assert_eq!(error.code, codes::DENIED, "{error}");

    // Emit: test.a reaches the owner, test.b is dropped and audited.
    t1.emit("test.b", "no").await.unwrap();
    t1.emit("test.a", "yes").await.unwrap();
    let event = tokio::time::timeout(TIMEOUT, events.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.subject, "test.a");
    assert_eq!(event.from, "token:t-1");
    wait_logged(
        &server,
        &[
            "bus audit: deny",
            "from=token:t-1",
            "kind=emit",
            "subject=test.b",
        ],
    )
    .await;

    // A forged core event, even with emit = ["*"]: nobody gets it. Events of one
    // sender arrive in order, so once `test.c` (sent after it) is in, a delivered
    // forgery would have come first.
    let mut all = owner.subscribe("*.>").await.unwrap();
    let (any, _) = mint(&owner, "t-3", Rights::default().emit(&["*"])).await;
    let t3 = participant(&server, &any, "t-3").await;
    t3.emit("pane.exited", "forged").await.unwrap();
    t3.emit("test.c", "after").await.unwrap();
    loop {
        let event = tokio::time::timeout(TIMEOUT, all.next())
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            event.subject, "pane.exited",
            "the forged event was delivered"
        );
        if event.subject == "test.c" {
            break;
        }
    }
    // The broker drops it before the authorizer (its own audit line).
    wait_logged(
        &server,
        &["tilrd::bus::audit", "namespace of tilrd", "pane.exited"],
    )
    .await;
}

/// A call that passed authorization and found no method: in this build the
/// methods of `bus-instances` and `bus-core-services` are not there yet (the broker
/// authorizes before it routes).
fn allowed_but_absent(result: Result<String, BusError>) {
    match result {
        Err(error) if error.code == codes::NOT_FOUND && error.message.starts_with("no method ") => {
        }
        other => panic!("expected the call to pass authorization, got {other:?}"),
    }
}

async fn revoke_id(bus: &Bus, revocation_id: &str) -> Result<wire::RevokeReply, BusError> {
    bus.call_typed::<_, wire::RevokeReply>(
        "bus.revoke",
        &wire::RevokeRequest {
            what: Some(wire::revoke_request::What::RevocationId(
                revocation_id.to_owned(),
            )),
        },
        CallOptions::default(),
    )
    .await
}

#[tokio::test]
async fn control_methods_need_control_rights() {
    let server = start();
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    // The owner's answer for each control method of a later change: passes
    // authorization, no provider yet.
    for method in [
        "bus.instances",
        "bus.methods",
        "bus.start",
        "bus.stop",
        "bus.reload",
        "bus.reload_config",
        "server.reload",
        "server.update",
    ] {
        allowed_but_absent(calls(&owner, method, "", CallOptions::default()).await);
    }

    let (none, _) = mint(&owner, "none", Rights::default().call(&["*"])).await;
    let bus = participant(&server, &none, "none").await;
    // `call = ["*"]` is not `bus.*`, and not server.reload or server.update.
    for method in [
        "bus.instances",
        "bus.methods",
        "bus.start",
        "bus.stop",
        "bus.reload",
        "bus.reload_config",
        "bus.token",
        "bus.revoke",
        "bus.rotate_key",
        "server.reload",
        "server.update",
    ] {
        denied(calls(&bus, method, "", CallOptions::default()).await);
    }

    // list: bus.instances and bus.methods pass, the rest not.
    let (list, _) = mint(&owner, "list", Rights::default().control(&["list"])).await;
    let bus = participant(&server, &list, "list").await;
    allowed_but_absent(calls(&bus, "bus.instances", "", CallOptions::default()).await);
    allowed_but_absent(calls(&bus, "bus.methods", "", CallOptions::default()).await);
    denied(calls(&bus, "bus.rotate_key", "", CallOptions::default()).await);

    // reload of one instance: bus.reload passes to the method (which checks the
    // instance it acts on), bus.stop not.
    let (reload, _) = mint(&owner, "r", Rights::default().control(&["reload:t-1"])).await;
    let bus = participant(&server, &reload, "r").await;
    allowed_but_absent(calls(&bus, "bus.reload", "", CallOptions::default()).await);
    denied(calls(&bus, "bus.stop", "", CallOptions::default()).await);
    // server.reload and server.update are actions of their own.
    let (server_rights, _) = mint(
        &owner,
        "s",
        Rights::default().control(&["server.reload", "server.update"]),
    )
    .await;
    let bus = participant(&server, &server_rights, "s").await;
    allowed_but_absent(calls(&bus, "server.reload", "", CallOptions::default()).await);
    allowed_but_absent(calls(&bus, "server.update", "", CallOptions::default()).await);
    denied(calls(&bus, "bus.reload", "", CallOptions::default()).await);

    // token: minting only rights it holds itself, under its own name.
    let (minter, _) = mint(
        &owner,
        "minter",
        Rights::default().control(&["token"]).call(&["test.*"]),
    )
    .await;
    let bus = participant(&server, &minter, "minter").await;
    let (child, _) = try_mint(&bus, "minter.child", Rights::default().call(&["test.*"]))
        .await
        .expect("a right the minter holds");
    calls(&owner, "test.echo", "c", with(&child)).await.unwrap();
    let error = try_mint(&bus, "minter.wide", Rights::default().call(&["*"]))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
    denied(calls(&bus, "server.reload", "", CallOptions::default()).await);

    // revoke: a minter without it changes nothing; one with it revokes.
    let (victim, victim_reply) = mint(&owner, "victim", Rights::default().call(&["test.*"])).await;
    let error = revoke_id(&bus, &victim_reply.revocation_id)
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
    calls(&owner, "test.echo", "v", with(&victim))
        .await
        .unwrap();
    let (revoker, _) = mint(&owner, "revoker", Rights::default().control(&["revoke"])).await;
    let revoker = participant(&server, &revoker, "revoker").await;
    revoke_id(&revoker, &victim_reply.revocation_id)
        .await
        .unwrap();
    let reason = denied(calls(&owner, "test.echo", "v", with(&victim)).await);
    assert!(reason.starts_with("revoked"), "{reason}");

    // rotate_key: refused without it (the key and the tokens stay), done with it.
    let key = std::fs::read(state_dir(&server).join("bus-root.key")).unwrap();
    denied(calls(&bus, "bus.rotate_key", "", CallOptions::default()).await);
    assert_eq!(
        std::fs::read(state_dir(&server).join("bus-root.key")).unwrap(),
        key
    );
    calls(&owner, "test.echo", "c", with(&child)).await.unwrap();
    // (Per call, so that the connection the rotation closes is not the caller's.)
    let (rotator, _) = mint(
        &owner,
        "rotator",
        Rights::default().control(&["rotate_key"]),
    )
    .await;
    owner
        .call_typed::<_, wire::RotateKeyReply>(
            "bus.rotate_key",
            &wire::RotateKeyRequest {},
            with(&rotator),
        )
        .await
        .unwrap();
    assert_ne!(
        std::fs::read(state_dir(&server).join("bus-root.key")).unwrap(),
        key
    );
    denied(calls(&owner, "test.echo", "c", with(&child)).await);
}

#[tokio::test]
async fn server_command_is_the_owners_on_the_unix_socket_only() {
    let server = start();
    let owner = server.owner().await;
    // The owner on the Unix socket: passes (no provider in this build).
    allowed_but_absent(calls(&owner, "server.command", "", CallOptions::default()).await);
    // The owner on TCP, with the access token: denied.
    let tcp_owner = Bus::connect(
        Transport::WebSocket {
            url: server.url(),
            access_token: Some(server.token()),
            credential: None,
            origin: None,
        },
        ConnectOptions::new("tcp-owner"),
    )
    .await
    .unwrap();
    assert_eq!(tcp_owner.identity(), "owner");
    let reason = denied(calls(&tcp_owner, "server.command", "", CallOptions::default()).await);
    assert_eq!(
        reason,
        "server.command is the owner's on the Unix socket only"
    );
    // A token on the Unix socket, whatever it lists, and as a per-call credential.
    let (every, _) = mint(
        &owner,
        "every",
        Rights::default()
            .call(&["*", "server.*", "server.command"])
            .control(&["*"]),
    )
    .await;
    let bus = participant(&server, &every, "every").await;
    denied(calls(&bus, "server.command", "", CallOptions::default()).await);
    denied(calls(&owner, "server.command", "", with(&every)).await);
    // server.update is a control action: `call` does not cover it, `control` does.
    allowed_but_absent(calls(&bus, "server.update", "", CallOptions::default()).await);
    let (calls_only, _) = mint(&owner, "c", Rights::default().call(&["*", "server.*"])).await;
    let bus = participant(&server, &calls_only, "c").await;
    denied(calls(&bus, "server.update", "", CallOptions::default()).await);
}

#[tokio::test]
async fn a_minted_token_lives_within_its_minter() {
    let server = start();
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    let (minter, minter_reply) = mint(
        &owner,
        "app",
        Rights::default()
            .control(&["token"])
            .call(&["test.*"])
            .ttl_ms(3_600_000),
    )
    .await;
    let bus = participant(&server, &minter, "app").await;
    // No longer than the minter's token, under its name.
    let (first, reply) = try_mint(&bus, "app.first", Rights::default().call(&["test.*"]))
        .await
        .unwrap();
    assert_eq!(reply.expires_at_ms, minter_reply.expires_at_ms);
    let (second, _) = try_mint(&bus, "app.second", Rights::default().call(&["test.*"]))
        .await
        .unwrap();
    let error = try_mint(&bus, "owner-like", Rights::default().call(&["test.*"]))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
    // A child revoked alone: the minter and the other child keep working.
    revoke_id(&owner, &reply.revocation_id).await.unwrap();
    let reason = denied(calls(&owner, "test.echo", "", with(&first)).await);
    assert!(reason.starts_with("revoked"), "{reason}");
    calls(&bus, "test.echo", "m", CallOptions::default())
        .await
        .unwrap();
    calls(&owner, "test.echo", "s", with(&second))
        .await
        .unwrap();
    // An attenuated minter mints nothing.
    let narrowed = minter.attenuate("check if true;").unwrap();
    let error = owner
        .call_typed::<_, wire::TokenReply>(
            "bus.token",
            &wire::TokenRequest {
                name: "app.n".into(),
                call: vec!["test.*".into()],
                ..wire::TokenRequest::default()
            },
            with(&narrowed),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
    assert!(error.message.contains("appended blocks"), "{error}");
    // The minter revoked: its children with it.
    revoke_id(&owner, &minter_reply.revocation_id)
        .await
        .unwrap();
    let reason = denied(calls(&owner, "test.echo", "", with(&second)).await);
    assert_eq!(reason, "revoked: the token of its minter was revoked");

    // A minter bound to an origin mints for that origin only.
    let (web, _) = mint(
        &owner,
        "web",
        Rights {
            origin: "tilr://app".into(),
            ..Rights::default().control(&["token"]).call(&["test.*"])
        },
    )
    .await;
    let page = Bus::connect(
        Transport::UnixWebSocket {
            path: server.socket(),
            credential: Some(web),
            origin: Some("tilr://app".into()),
        },
        ConnectOptions::new("page"),
    )
    .await
    .unwrap();
    let (bound, _) = try_mint(&page, "web.child", Rights::default().call(&["test.*"]))
        .await
        .unwrap();
    let error = connect(&server, &bound, ConnectOptions::new("elsewhere"))
        .await
        .unwrap_err();
    assert!(error.message.starts_with("origin"), "{error}");
    let error = try_mint(
        &page,
        "web.other",
        Rights {
            origin: "http://localhost:7891".into(),
            ..Rights::default().call(&["test.*"])
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, codes::DENIED, "{error}");
}

#[tokio::test]
async fn a_token_of_another_server_is_refused() {
    let a = start();
    let b = start();
    let owner_a = a.owner().await;
    let owner_b = b.owner().await;
    let _provider = echo_provider(&b, "echo").await;
    let (token, _) = mint(&owner_a, "x", Rights::default().call(&["test.*"])).await;
    let reason = denied(calls(&owner_b, "test.echo", "x", with(&token)).await);
    assert_eq!(reason, "not a token of this server");
    let error = connect(&b, &token, ConnectOptions::new("x"))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED);
    // Malformed: the same.
    let garbage = Credential::from_bytes(Bytes::from_static(b"\x01\x02\x03"));
    denied(calls(&owner_b, "test.echo", "x", with(&garbage)).await);
    let error = connect(&b, &garbage, ConnectOptions::new("g"))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::DENIED);
}

#[tokio::test]
async fn a_per_call_credential_is_a_bearer() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    // Narrow caller, wide credential.
    let (bare, _) = mint(&owner, "t-1", Rights::default()).await;
    let t1 = participant(&server, &bare, "t-1").await;
    denied(calls(&t1, "test.from", "", CallOptions::default()).await);
    let (bearer, _) = mint(&owner, "bearer", Rights::default().call(&["test.from"])).await;
    assert_eq!(
        calls(&t1, "test.from", "", with(&bearer)).await.unwrap(),
        "token:t-1",
        "from stays the caller"
    );
    // Wide caller, narrow credential.
    let (echo_only, _) = mint(&owner, "echo", Rights::default().call(&["test.echo"])).await;
    denied(calls(&owner, "test.whoami", "", with(&echo_only)).await);
    assert_eq!(
        calls(&owner, "test.echo", "e", with(&echo_only))
            .await
            .unwrap(),
        "e"
    );
    // The audit line names the caller and the bearer's participant.
    wait_logged(
        &server,
        &[
            "bus audit: deny",
            "from=owner bearer=token:echo",
            "method=test.whoami",
        ],
    )
    .await;
}

#[tokio::test]
async fn an_attenuated_token_only_narrows() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    let _t3 = runtime(&server, &owner, "t-3", Rights::default()).await;
    let (token, _) = mint(&owner, "t-1", Rights::default().call(&["test.*"])).await;
    let t1 = participant(&server, &token, "t-1").await;

    // Narrowed to one target, without the server.
    let narrowed = token.attenuate(r#"check if target("token:t-2");"#).unwrap();
    assert_eq!(
        calls(&t1, "test.echo", "x", with(&narrowed).target("token:t-2"))
            .await
            .unwrap(),
        "x"
    );
    let reason = denied(calls(&t1, "test.echo", "x", with(&narrowed).target("token:t-3")).await);
    assert!(reason.contains("a check of the token"), "{reason}");
    // A narrowed token says Hello as its participant and stays narrow.
    let narrow_bus = participant(&server, &narrowed, "t-1n").await;
    assert_eq!(narrow_bus.identity(), "token:t-1");
    denied(calls(&narrow_bus, "test.echo", "x", to("token:t-3")).await);

    // A block that adds a right adds nothing.
    let (echo, _) = mint(&owner, "e", Rights::default().call(&["test.echo"])).await;
    let widened = echo
        .attenuate(r#"call_any(true); call("test.whoami"); control_all(true);"#)
        .unwrap();
    denied(calls(&owner, "test.whoami", "", with(&widened)).await);
    denied(calls(&owner, "bus.rotate_key", "", with(&widened)).await);
    calls(&owner, "test.echo", "", with(&widened))
        .await
        .unwrap();

    // More than 8 blocks or 16 KiB: denied.
    let mut long = echo.clone();
    for _ in 0..8 {
        long = long.attenuate("check if true;").unwrap();
    }
    let reason = denied(calls(&owner, "test.echo", "", with(&long)).await);
    assert!(reason.contains("8 blocks"), "{reason}");
    let big = echo
        .attenuate(&format!(
            "check if true or \"{}\" == \"x\";",
            "a".repeat(17 * 1024)
        ))
        .unwrap();
    let reason = denied(calls(&owner, "test.echo", "", with(&big)).await);
    assert!(reason.contains("16 KiB"), "{reason}");
}

#[tokio::test]
async fn authorization_that_runs_too_long_is_denied() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    let (token, _) = mint(&owner, "t-1", Rights::default().call(&["test.*"])).await;
    // 10 x 10 x 10 derived facts: over the 1000-fact limit.
    let heavy = token
        .attenuate(
            "n(0); n(1); n(2); n(3); n(4); n(5); n(6); n(7); n(8); n(9);
             p($a, $b, $c) <- n($a), n($b), n($c);
             check if p(0, 0, 0);",
        )
        .unwrap();
    let reason = denied(calls(&owner, "test.echo", "", with(&heavy)).await);
    assert_eq!(reason, "authorization limit reached");
    // The token itself still works.
    calls(&owner, "test.echo", "", with(&token)).await.unwrap();
}

#[tokio::test]
async fn a_revocation_clears_cached_decisions() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    let (token, reply) = mint(&owner, "t-1", Rights::default().call(&["test.*"])).await;
    // The same decision ten times: answered from the cache.
    for _ in 0..10 {
        calls(&owner, "test.echo", "", with(&token)).await.unwrap();
    }
    // Revoked by its id: the next call is denied, inside the cache's second.
    owner
        .call_typed::<_, wire::RevokeReply>(
            "bus.revoke",
            &wire::RevokeRequest {
                what: Some(wire::revoke_request::What::RevocationId(
                    reply.revocation_id,
                )),
            },
            CallOptions::default(),
        )
        .await
        .unwrap();
    let reason = denied(calls(&owner, "test.echo", "", with(&token)).await);
    assert!(reason.starts_with("revoked"), "{reason}");
    // A key rotation too.
    let (fresh, _) = mint(&owner, "t-1", Rights::default().call(&["test.*"])).await;
    calls(&owner, "test.echo", "", with(&fresh)).await.unwrap();
    owner
        .call_typed::<_, wire::RotateKeyReply>(
            "bus.rotate_key",
            &wire::RotateKeyRequest {},
            CallOptions::default(),
        )
        .await
        .unwrap();
    denied(calls(&owner, "test.echo", "", with(&fresh)).await);
}

#[tokio::test]
async fn a_revoked_token_stops_working_at_once() {
    let server = start();
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    let (token, _) = mint(&owner, "t-1", Rights::default().call(&["test.*"])).await;
    let narrowed = token.attenuate(r#"check if method("test.echo");"#).unwrap();
    let holder = participant(&server, &token, "t-1").await;
    for _ in 0..10 {
        calls(&owner, "test.echo", "", with(&token)).await.unwrap();
    }
    calls(&owner, "test.echo", "", with(&narrowed))
        .await
        .unwrap();
    owner
        .call_typed::<_, wire::RevokeReply>(
            "bus.revoke",
            &wire::RevokeRequest {
                what: Some(wire::revoke_request::What::Credential(
                    token.as_bytes().clone(),
                )),
            },
            CallOptions::default(),
        )
        .await
        .unwrap();
    for credential in [&token, &narrowed] {
        let reason = denied(calls(&owner, "test.echo", "", with(credential)).await);
        assert!(reason.starts_with("revoked"), "{reason}");
    }
    // The connection that said Hello with it is closed with 4003.
    assert_eq!(close_code(&holder).await, Some(tilr_bus::close::CREDENTIAL));
    let error = connect(&server, &token, ConnectOptions::new("again"))
        .await
        .unwrap_err();
    assert!(error.message.starts_with("revoked"), "{error}");

    // And after a restart.
    drop(owner);
    let (server, _kept) = restart(server, BusOptions::default());
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    let reason = denied(calls(&owner, "test.echo", "", with(&token)).await);
    assert!(reason.starts_with("revoked"), "{reason}");
}

#[tokio::test]
async fn rotating_the_root_key_ends_every_token() {
    let server = start();
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    let key = std::fs::read(state_dir(&server).join("bus-root.key")).unwrap();
    let (token, _) = mint(&owner, "web", Rights::default().call(&["test.*"])).await;
    let holder = participant(&server, &token, "web").await;
    calls(&holder, "test.echo", "", CallOptions::default())
        .await
        .unwrap();
    owner
        .call_typed::<_, wire::RotateKeyReply>(
            "bus.rotate_key",
            &wire::RotateKeyRequest {},
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_ne!(
        std::fs::read(state_dir(&server).join("bus-root.key")).unwrap(),
        key
    );
    let reason = denied(calls(&owner, "test.echo", "", with(&token)).await);
    assert!(reason.contains("not a token of this server"), "{reason}");
    assert_eq!(close_code(&holder).await, Some(tilr_bus::close::CREDENTIAL));
    // A token of the new key works; the owner was never affected.
    let (fresh, _) = mint(&owner, "web", Rights::default().call(&["test.*"])).await;
    let holder = participant(&server, &fresh, "web").await;
    calls(&holder, "test.echo", "", CallOptions::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn denied_and_control_calls_are_audited() {
    let server = start_for_runtime();
    let owner = server.owner().await;
    let _t2 = runtime(&server, &owner, "t-2", Rights::default()).await;
    let (token, reply) = mint(&owner, "t-1", Rights::default().call(&["test.echo"])).await;
    let t1 = participant(&server, &token, "t-1").await;
    denied(calls(&t1, "test.whoami", "", CallOptions::default()).await);
    let line = wait_logged(
        &server,
        &["bus audit: deny", "from=token:t-1", "method=test.whoami"],
    )
    .await;
    assert!(
        line.contains("reason=token:t-1 has no right to call test.whoami"),
        "{line}"
    );
    assert!(
        line.contains(&format!("rid={}", &reply.revocation_id[..16])),
        "{line}"
    );
    wait_logged(
        &server,
        &["bus audit: mint", "from=owner", "participant=token:t-1"],
    )
    .await;
    wait_logged(
        &server,
        &["bus audit: control allow", "from=owner", "action=token"],
    )
    .await;
    // Denied control calls too.
    denied(calls(&t1, "bus.rotate_key", "", CallOptions::default()).await);
    wait_logged(
        &server,
        &["bus audit: deny", "from=token:t-1", "action=rotate_key"],
    )
    .await;
    owner
        .call_typed::<_, wire::RevokeReply>(
            "bus.revoke",
            &wire::RevokeRequest {
                what: Some(wire::revoke_request::What::RevocationId(
                    reply.revocation_id.clone(),
                )),
            },
            CallOptions::default(),
        )
        .await
        .unwrap();
    wait_logged(&server, &["bus audit: revoke", "from=owner"]).await;
    // No line holds the token, in any of its spellings.
    let spellings = [
        token.to_base64url(),
        text(token.as_bytes()),
        token
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    ];
    for line in server.log_lines() {
        for spelling in &spellings {
            assert!(
                !line.contains(spelling.as_str()),
                "a token in the log: {line}"
            );
        }
    }
    assert!(!audit_lines(&server).is_empty());
}

// ---------------------------------------------------------------------------
// Controlled time (the test clock of `failpoints` builds)
// ---------------------------------------------------------------------------

/// 2026-11-17 00:00 UTC.
#[cfg(feature = "failpoints")]
const START_US: u64 = 1_795_392_000_000_000;

#[cfg(feature = "failpoints")]
fn start_at(clock: &TestClock) -> BusServer {
    BusServer::start(BusOptions {
        env: vec![clock.env()],
        ..BusOptions::default()
    })
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_expired_token_is_refused() {
    let clock = TestClock::new(START_US);
    let server = start_at(&clock);
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    let (token, reply) = mint(
        &owner,
        "t-1",
        Rights::default().call(&["test.*"]).ttl_ms(3_600_000),
    )
    .await;
    assert_eq!(reply.expires_at_ms, START_US / 1000 + 3_600_000);
    let holder = participant(&server, &token, "t-1").await;
    calls(&holder, "test.echo", "", CallOptions::default())
        .await
        .unwrap();
    calls(&owner, "test.echo", "", with(&token)).await.unwrap();
    // A second past the expiry: denied, cached decision or not.
    clock.set(START_US + HOUR_US + 1_000_000);
    let reason = denied(calls(&owner, "test.echo", "", with(&token)).await);
    assert!(reason.starts_with("expired"), "{reason}");
    // The connection that holds it is closed with 4003.
    assert_eq!(close_code(&holder).await, Some(tilr_bus::close::CREDENTIAL));
    let error = connect(&server, &token, ConnectOptions::new("late"))
        .await
        .unwrap_err();
    assert!(error.message.starts_with("expired"), "{error}");
    // A ttl over 24 h is refused.
    let error = try_mint(&owner, "t-2", Rights::default().ttl_ms(25 * 3_600_000))
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_outside_token_is_renewed_on_its_connection() {
    let clock = TestClock::new(START_US);
    let server = start_at(&clock);
    let owner = server.owner().await;
    let _provider = echo_provider(&server, "echo").await;
    let (first, _) = mint(&owner, "web", Rights::default().call(&["test.*"])).await;
    let renewing = participant(&server, &first, "renewing").await;
    let (other, _) = mint(&owner, "lazy", Rights::default().call(&["test.*"])).await;
    let lazy = participant(&server, &other, "lazy").await;

    // An attenuated token is not renewed.
    let narrowed = first.attenuate(r#"check if method("test.echo");"#).unwrap();
    let error = renewing
        .call_typed::<_, wire::RenewReply>(
            "bus.renew",
            &wire::RenewRequest {
                credential: narrowed.as_bytes().clone(),
            },
            CallOptions::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST);
    assert!(
        error
            .message
            .contains("renew the token it was attenuated from"),
        "{error}"
    );

    // 20 h in: renewed and presented on the connection.
    clock.set(START_US + 20 * HOUR_US);
    let renewed: wire::RenewReply = renewing
        .call_typed(
            "bus.renew",
            &wire::RenewRequest::default(),
            CallOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(renewed.expires_at_ms, (START_US + 44 * HOUR_US) / 1000);
    renewing
        .update_credential(Credential::from_bytes(renewed.credential.clone()))
        .await
        .unwrap();
    calls(&renewing, "test.echo", "", CallOptions::default())
        .await
        .unwrap();
    let reason = denied(calls(&owner, "test.echo", "", with(&first)).await);
    assert!(reason.starts_with("revoked"), "{reason}");

    // 30 h in: the renewed connection is open, the other closed with 4003.
    clock.set(START_US + 30 * HOUR_US);
    assert_eq!(close_code(&lazy).await, Some(tilr_bus::close::CREDENTIAL));
    assert!(!renewing.is_closed());
    calls(&renewing, "test.echo", "", CallOptions::default())
        .await
        .unwrap();
    // A token of another participant is not taken as an update.
    let (stranger, _) = mint(&owner, "stranger", Rights::default()).await;
    renewing.update_credential(stranger).await.unwrap();
    tokio::time::timeout(TIMEOUT, renewing.closed())
        .await
        .expect("a refused update closes the connection");
    wait_logged(
        &server,
        &[
            "bus audit: deny",
            "kind=credential_update",
            "names token:stranger",
        ],
    )
    .await;
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_renewal_comes_in_the_second_half_and_stops_at_the_minters_expiry() {
    let clock = TestClock::new(START_US);
    let server = start_at(&clock);
    let owner = server.owner().await;
    let renew = |bus: Bus| async move {
        bus.call_typed::<_, wire::RenewReply>(
            "bus.renew",
            &wire::RenewRequest::default(),
            CallOptions::default(),
        )
        .await
    };
    let (day, _) = mint(&owner, "day", Rights::default().call(&["test.*"])).await;
    let (minter, minter_reply) = mint(
        &owner,
        "m",
        Rights::default()
            .control(&["token"])
            .call(&["test.*"])
            .ttl_ms(2 * 3_600_000),
    )
    .await;
    let minter = participant(&server, &minter, "m").await;
    // Asked for a day, given the minter's two hours.
    let (child, reply) = try_mint(&minter, "m.child", Rights::default().call(&["test.*"]))
        .await
        .unwrap();
    assert_eq!(reply.expires_at_ms, minter_reply.expires_at_ms);
    assert_eq!(reply.expires_at_ms, START_US / 1000 + 2 * 3_600_000);

    clock.set(START_US + 3 * HOUR_US / 2);
    // The child is in its second half, but already lives as long as its minter.
    let error = renew(participant(&server, &child, "child").await)
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST);
    assert!(
        error.message.contains("as long as the token of its minter"),
        "{error}"
    );
    // A day's token an hour and a half in: too early.
    let error = renew(participant(&server, &day, "day").await)
        .await
        .unwrap_err();
    assert_eq!(error.code, codes::BAD_REQUEST);
    assert!(error.message.starts_with("too early"), "{error}");
}
