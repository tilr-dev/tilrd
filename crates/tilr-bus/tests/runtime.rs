//! The test runtime (`examples/test_runtime.rs`), built with the tests, driven over an
//! instance socket by a partner in the test that plays the broker.

mod common;

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use bytes::Bytes;
use common::{Peer, ack};
use tilr_bus::bus::{Call, Cancel, Credit, Kind, Lane, envelope::Msg, reply};
use tilr_bus::codes;
use tokio::process::{Child, Command};

/// `target/<profile>/examples/test_runtime`: the nearest `examples/test_runtime` above this
/// test's executable, which is `target/<profile>/deps/` before Cargo 1.100 and
/// `target/<profile>/build/tilr-bus/<hash>/out/` from 1.100 on.
fn runtime_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let name = format!("test_runtime{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .map(|dir| dir.join("examples").join(&name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            panic!(
                "no examples/{name} above {}: cargo build -p tilr-bus --example test_runtime \
                 (cargo nextest run -p tilr-bus builds it)",
                exe.display()
            )
        })
}

struct Runtime {
    child: Child,
    dir: PathBuf,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Start the runtime and take its connection. The connection is awaited as an event,
/// with no deadline: how long a freshly built binary takes to start is not part of a
/// check's time window (decision 2026-10-21, as in the tilrd tests); a runtime that
/// exits first fails the test with its stderr. `WAIT` bounds everything after it.
async fn start(name: &str, extra: &[(&str, &str)]) -> (Runtime, Peer) {
    let dir = std::env::temp_dir().join(format!("tilr-bus-rt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bus.sock");
    let stderr_path = dir.join("stderr");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let mut command = Command::new(runtime_path());
    command
        .env_clear()
        .env("TILR_BUS_SOCK", &path)
        .env("TILR_BUS_TOKEN", "instance-token-of-t")
        .env("TILR_INSTANCE", name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&stderr_path).unwrap())
        .kill_on_drop(true);
    for (key, value) in extra {
        command.env(key, value);
    }
    let child = command.spawn().unwrap();
    let mut runtime = Runtime { child, dir };
    let stream = tokio::select! {
        accepted = listener.accept() => accepted.unwrap().0,
        status = runtime.child.wait() => panic!(
            "the runtime exited before it connected ({status:?}); its stderr:\n{}",
            std::fs::read_to_string(&stderr_path).unwrap_or_default()
        ),
    };
    (runtime, Peer::from_unix(stream))
}

async fn call(
    peer: &mut Peer,
    id: u64,
    method: &str,
    payload: &'static str,
) -> Result<Bytes, (String, String)> {
    peer.send(Msg::Call(Call {
        id,
        method: method.into(),
        payload: Bytes::from_static(payload.as_bytes()),
        from: "owner".into(),
        ..Default::default()
    }))
    .await;
    loop {
        match peer.recv().await {
            Msg::Reply(reply) if reply.id == id => {
                return match reply.result {
                    Some(reply::Result::Payload(payload)) => Ok(payload),
                    Some(reply::Result::Error(error)) => Err((error.code, error.message)),
                    None => Ok(Bytes::new()),
                };
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn the_test_runtime_provides_its_methods_over_an_instance_socket() {
    let (_runtime, mut peer) = start("t-1", &[]).await;
    let hello = peer.greet(ack("instance:t-1")).await;
    assert_eq!(hello.instance_token, "instance-token-of-t");
    assert_eq!(hello.client_name, "t-1");
    let provided: Vec<(&str, Kind, Lane)> = hello
        .provides
        .iter()
        .map(|spec| (spec.method.as_str(), spec.kind(), spec.lane()))
        .collect();
    for method in [
        "test.echo",
        "test.whoami",
        "test.from",
        "test.env",
        "test.sleep",
        "test.nested",
        "test.deadline",
        "test.close_bus",
        "test.record",
        "test.records",
        "test.started",
        "test.fail",
    ] {
        assert!(
            provided.contains(&(method, Kind::Unary, Lane::Interactive)),
            "{method}"
        );
    }
    assert!(provided.contains(&("test.count", Kind::Stream, Lane::Interactive)));
    assert!(provided.contains(&("test.flood", Kind::Stream, Lane::Interactive)));
    assert!(provided.contains(&("test.bulk", Kind::Stream, Lane::Bulk)));

    assert_eq!(call(&mut peer, 1, "test.echo", "hi").await.unwrap(), "hi");
    assert_eq!(
        call(&mut peer, 2, "test.whoami", "").await.unwrap(),
        "instance:t-1"
    );
    assert_eq!(call(&mut peer, 3, "test.from", "").await.unwrap(), "owner");
    assert_eq!(
        call(&mut peer, 4, "test.fail", "").await.unwrap_err(),
        (codes::BAD_REQUEST.into(), "x".into())
    );
    let (code, _) = call(&mut peer, 5, "test.crash", "").await.unwrap_err();
    assert_eq!(code, codes::HANDLER_ERROR);
    assert_eq!(call(&mut peer, 6, "test.record", "a").await.unwrap(), "1");
    assert_eq!(call(&mut peer, 7, "test.record", "b").await.unwrap(), "2");
    let env = call(&mut peer, 8, "test.env", "").await.unwrap();
    let env = String::from_utf8_lossy(&env);
    assert!(env.contains(r#""TILR_INSTANCE":"t-1""#), "{env}");
    assert!(env.contains(r#""fds":[0,1,2"#), "{env}");

    // test.deadline answers the timeout_ms it got.
    peer.send(Msg::Call(Call {
        id: 9,
        method: "test.deadline".into(),
        timeout_ms: 600,
        ..Default::default()
    }))
    .await;
    let Msg::Reply(reply) = peer.recv().await else {
        panic!("no reply");
    };
    assert_eq!(
        reply.result,
        Some(reply::Result::Payload(Bytes::from_static(b"600")))
    );

    // test.count streams 1..n then End.
    peer.send(Msg::Call(Call {
        id: 10,
        method: "test.count".into(),
        payload: Bytes::from_static(b"3"),
        ..Default::default()
    }))
    .await;
    for n in ["1", "2", "3"] {
        let Msg::Chunk(chunk) = peer.recv().await else {
            panic!("no chunk");
        };
        assert_eq!(chunk.payload, n);
    }
    let Msg::End(end) = peer.recv().await else {
        panic!("no end");
    };
    assert_eq!(end.id, 10);

    // A cancelled sleep is recorded.
    peer.send(Msg::Call(Call {
        id: 11,
        method: "test.sleep".into(),
        payload: Bytes::from_static(b"10000"),
        ..Default::default()
    }))
    .await;
    // Once it has started, the cancel aborts a running handler, whose guard records it.
    assert_eq!(
        call(&mut peer, 12, "test.started", "11").await.unwrap(),
        "started"
    );
    peer.send(Msg::Cancel(Cancel {
        id: 11,
        reason: "timeout".into(),
    }))
    .await;
    // test.records 3 answers once the third record, the abort, is in.
    assert_eq!(
        call(&mut peer, 13, "test.records", "3").await.unwrap(),
        "a\nb\naborted test.sleep 11"
    );
}

#[tokio::test]
async fn the_test_runtimes_flood_waits_for_credit() {
    let (_runtime, mut peer) = start("t-2", &[]).await;
    peer.greet(ack("instance:t-2")).await;
    peer.send(Msg::Call(Call {
        id: 1,
        method: "test.flood".into(),
        payload: Bytes::from_static(b"100000"),
        window: 65536,
        ..Default::default()
    }))
    .await;
    let mut received = 0usize;
    while received < 65536 {
        let Msg::Chunk(chunk) = peer.recv().await else {
            panic!("no chunk");
        };
        received += chunk.payload.len();
    }
    assert_eq!(received, 65536);
    assert!(
        peer.quiet(Duration::from_millis(200)).await,
        "sent past the credit"
    );
    peer.send(Msg::Credit(Credit {
        id: 1,
        bytes: 65536,
    }))
    .await;
    loop {
        match peer.recv().await {
            Msg::Chunk(chunk) => received += chunk.payload.len(),
            Msg::End(end) => {
                assert_eq!(end.id, 1);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(received, 100000);
}

#[tokio::test]
async fn the_test_runtime_without_hello_says_nothing() {
    let (_runtime, mut peer) = start("t-3", &[("TEST_NO_HELLO", "1")]).await;
    assert!(peer.quiet(Duration::from_millis(300)).await);
}
