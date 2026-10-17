//! Exec over the protocol (UC-077): run a program on the server's machine
//! without a shell or a pane, its stdout and stderr apart, its exit code or
//! signal, a timeout, cancelling, stdin. tilr.proto has no such messages
//! yet: these are the driver's draft of them, on field numbers tilr.proto
//! does not use, sent only to a server that offers the "exec" feature. They
//! go once the real ones exist.

use std::time::Instant;

use bytes::Bytes;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::session::ms;
use crate::transfer::hex;

/// The HelloAck feature a server announces Exec with.
pub const FEATURE: &str = "exec";

/// Start a program: `argv[0]` looked up in PATH, no shell.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecStart {
    #[prost(uint64, tag = "1")]
    pub request_id: u64,
    #[prost(string, repeated, tag = "2")]
    pub argv: Vec<String>,
    /// Empty: the home directory.
    #[prost(string, tag = "3")]
    pub cwd: String,
    /// `NAME=value`, added to the server's environment.
    #[prost(string, repeated, tag = "4")]
    pub env: Vec<String>,
    /// The program's stdin is fed by ExecInput; without, it reads nothing.
    #[prost(bool, tag = "5")]
    pub stdin: bool,
    /// Killed (SIGKILL) after this long; 0: no limit.
    #[prost(uint32, tag = "6")]
    pub timeout_ms: u32,
}

/// Bytes for the program's stdin; `eof` closes it.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecInput {
    #[prost(uint64, tag = "1")]
    pub exec_id: u64,
    #[prost(bytes = "bytes", tag = "2")]
    pub data: Bytes,
    #[prost(bool, tag = "3")]
    pub eof: bool,
}

/// Send the program a signal (its number).
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecSignal {
    #[prost(uint64, tag = "1")]
    pub exec_id: u64,
    #[prost(int32, tag = "2")]
    pub signal: i32,
}

/// The client no longer wants it: the program and its children are killed.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecCancel {
    #[prost(uint64, tag = "1")]
    pub exec_id: u64,
}

/// Answer to ExecStart.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecStarted {
    #[prost(uint64, tag = "1")]
    pub request_id: u64,
    #[prost(uint64, tag = "2")]
    pub exec_id: u64,
    #[prost(uint32, tag = "3")]
    pub pid: u32,
    /// Why it did not start (no such program, say); then no ExecExit.
    #[prost(string, tag = "4")]
    pub error: String,
}

/// Output, in order per stream: 1 stdout, 2 stderr.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecOutput {
    #[prost(uint64, tag = "1")]
    pub exec_id: u64,
    #[prost(uint32, tag = "2")]
    pub stream: u32,
    #[prost(bytes = "bytes", tag = "3")]
    pub data: Bytes,
}

/// The program ended; after all its output.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecExit {
    #[prost(uint64, tag = "1")]
    pub exec_id: u64,
    /// Exit status when it exited.
    #[prost(int32, tag = "2")]
    pub code: i32,
    /// The signal that killed it, 0 if none.
    #[prost(int32, tag = "3")]
    pub signal: i32,
    #[prost(bool, tag = "4")]
    pub timed_out: bool,
    #[prost(bool, tag = "5")]
    pub cancelled: bool,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Client {
    #[prost(message, tag = "26")]
    Start(ExecStart),
    #[prost(message, tag = "27")]
    Input(ExecInput),
    #[prost(message, tag = "28")]
    Signal(ExecSignal),
    #[prost(message, tag = "29")]
    Cancel(ExecCancel),
}

/// A ClientMessage carrying a draft message (the oneof of tilr.proto
/// extended), with its input seq.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ClientMessage {
    #[prost(oneof = "Client", tags = "26, 27, 28, 29")]
    pub msg: Option<Client>,
    #[prost(uint64, tag = "100")]
    pub seq: u64,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Server {
    #[prost(message, tag = "18")]
    Started(ExecStarted),
    #[prost(message, tag = "19")]
    Output(ExecOutput),
    #[prost(message, tag = "20")]
    Exit(ExecExit),
}

/// What of a ServerMessage the driver reads as a draft message.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerMessage {
    #[prost(oneof = "Server", tags = "18, 19, 20")]
    pub msg: Option<Server>,
}

/// Output kept as text per stream; the rest is only counted and hashed.
const KEEP: usize = 256 * 1024;

/// One program run by Exec, as the driver follows it.
pub struct Run {
    pub id: u64,
    pub background: bool,
    started: Instant,
    out: [Vec<u8>; 2],
    bytes: [u64; 2],
    sha: [Sha256; 2],
    pub exit: Option<Value>,
}

impl Run {
    pub fn new(id: u64, background: bool) -> Self {
        Self {
            id,
            background,
            started: Instant::now(),
            out: [Vec::new(), Vec::new()],
            bytes: [0, 0],
            sha: [Sha256::new(), Sha256::new()],
            exit: None,
        }
    }

    pub fn output(&mut self, output: &ExecOutput) {
        let n = if output.stream == 2 { 1 } else { 0 };
        self.bytes[n] += output.data.len() as u64;
        self.sha[n].update(&output.data);
        let room = KEEP.saturating_sub(self.out[n].len());
        self.out[n].extend_from_slice(&output.data[..output.data.len().min(room)]);
    }

    pub fn exited(&mut self, exit: &ExecExit) -> Value {
        let value = json!({
            "exec": self.id, "code": exit.code, "signal": exit.signal,
            "timed_out": exit.timed_out, "cancelled": exit.cancelled,
            "stdout": String::from_utf8_lossy(&self.out[0]),
            "stderr": String::from_utf8_lossy(&self.out[1]),
            "stdout_bytes": self.bytes[0], "stderr_bytes": self.bytes[1],
            "stdout_sha256": hex(&std::mem::take(&mut self.sha[0]).finalize()),
            "stderr_sha256": hex(&std::mem::take(&mut self.sha[1]).finalize()),
            "ms": ms(self.started.elapsed()),
        });
        self.exit = Some(value.clone());
        value
    }
}
