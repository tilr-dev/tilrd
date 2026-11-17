//! Helpers of `bus-core-services` for the bus tests, beside `bus.rs`: a whole
//! stream read, the method listing, a FIFO whose writer's end the test waits
//! for, and a program run with `exec.run`.
//!
//! Owner: `bus-core-services`.

#![allow(
    dead_code,
    reason = "each test binary that declares `mod common` uses some of these"
)]

use std::io::Read;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use prost::Message;
use tilr_bus::bus::{MethodInfo, MethodsReply, MethodsRequest};
use tilr_bus::exec::{ExecExit, RunChunk, RunRequest, run_chunk};
use tilr_bus::{Bus, BusError, CallOptions, ChunkStream, StreamOptions};
use tokio::sync::oneshot;

use super::bus_streams::within;

/// What a stream gave: its chunks' payloads, then its `End` payload or error.
pub struct Streamed {
    pub chunks: Vec<Bytes>,
    pub end: Result<Bytes, BusError>,
}

/// Read a stream to its end.
pub async fn read_all(mut stream: ChunkStream) -> Streamed {
    let mut chunks = Vec::new();
    loop {
        match within(stream.next()).await {
            Some(Ok(chunk)) => chunks.push(chunk.bytes().expect("chunk payload")),
            Some(Err(err)) => {
                return Streamed {
                    chunks,
                    end: Err(err),
                };
            }
            None => {
                let end = stream.end_payload().cloned().unwrap_or_default();
                return Streamed {
                    chunks,
                    end: Ok(end),
                };
            }
        }
    }
}

/// Open the stream `method` with `request` and a `window` (0: the default),
/// and read it to its end.
pub async fn stream_all(bus: &Bus, method: &str, request: &impl Message, window: u32) -> Streamed {
    let stream = within(bus.stream(
        method,
        request.encode_to_vec(),
        StreamOptions::default().window(window),
    ))
    .await;
    match stream {
        Ok(stream) => read_all(stream).await,
        Err(err) => Streamed {
            chunks: Vec::new(),
            end: Err(err),
        },
    }
}

/// `bus.methods` as the owner.
pub async fn methods(bus: &Bus) -> Vec<MethodInfo> {
    within(bus.call_typed::<_, MethodsReply>(
        "bus.methods",
        &MethodsRequest {},
        CallOptions::default(),
    ))
    .await
    .expect("bus.methods")
    .methods
}

/// A call with protobuf messages, bounded.
pub async fn call<Req: Message, Resp: Message + Default>(
    bus: &Bus,
    method: &str,
    request: &Req,
) -> Result<Resp, BusError> {
    within(bus.call_typed(method, request, CallOptions::default())).await
}

/// A FIFO at `path`; the receiver hears once every writer closed it after the
/// first opened it: a program that held it open has ended.
pub fn fifo_closed(path: &Path) -> oneshot::Receiver<Vec<u8>> {
    let made = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo");
    assert!(made.success(), "mkfifo {}", path.display());
    let (tx, rx) = oneshot::channel();
    let fifo = path.to_owned();
    // A thread of its own: the open waits for the writer, the read for its end.
    std::thread::spawn(move || {
        let mut read = Vec::new();
        if let Ok(mut file) = std::fs::File::open(&fifo) {
            let _ = file.read_to_end(&mut read);
        }
        let _ = tx.send(read);
    });
    rx
}

/// The chunks of `exec.run` and its end.
pub struct Ran {
    pub exec_id: u64,
    pub pid: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: ExecExit,
}

/// What a stream of `exec.run` (or `server.command`, `server.update`) gave.
pub fn ran(streamed: Streamed) -> Ran {
    ran_after(streamed, None)
}

/// [`ran`] of the rest of a stream whose `started` (exec id, pid) was read.
pub fn ran_after(streamed: Streamed, started: Option<(u64, u32)>) -> Ran {
    let end = streamed
        .end
        .expect("the program's stream ended with an error");
    let (exec_id, pid) = started.unwrap_or_default();
    let mut ran = Ran {
        exec_id,
        pid,
        stdout: Vec::new(),
        stderr: Vec::new(),
        exit: ExecExit::decode(end).expect("ExecExit"),
    };
    for (n, chunk) in streamed.chunks.into_iter().enumerate() {
        match RunChunk::decode(chunk).expect("RunChunk").chunk {
            Some(run_chunk::Chunk::Started(started)) => {
                assert_eq!((n, ran.exec_id), (0, 0), "started comes first, once");
                ran.exec_id = started.exec_id;
                ran.pid = started.pid;
            }
            Some(run_chunk::Chunk::Output(output)) => {
                assert_eq!(output.exec_id, ran.exec_id);
                match output.stream {
                    1 => ran.stdout.extend_from_slice(&output.data),
                    2 => ran.stderr.extend_from_slice(&output.data),
                    other => panic!("output of stream {other}"),
                }
            }
            None => panic!("an empty RunChunk"),
        }
    }
    ran
}

/// `exec.run` of `argv`, read to its end.
pub async fn run(bus: &Bus, argv: &[&str]) -> Ran {
    let request = RunRequest {
        argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
        ..Default::default()
    };
    ran(stream_all(bus, "exec.run", &request, 0).await)
}

/// The first chunk of an `exec.run` stream: its `started`.
pub async fn started(stream: &mut ChunkStream) -> (u64, u32) {
    let chunk = within(stream.next())
        .await
        .expect("the stream ended before started")
        .expect("exec.run did not start");
    match RunChunk::decode(chunk.bytes().unwrap()).unwrap().chunk {
        Some(run_chunk::Chunk::Started(started)) => (started.exec_id, started.pid),
        other => panic!("first chunk {other:?}"),
    }
}

/// A path under the server's home directory, without links (as `fs.*`
/// answers paths).
pub fn real(dir: &Path, name: &str) -> PathBuf {
    std::fs::canonicalize(dir).unwrap().join(name)
}
