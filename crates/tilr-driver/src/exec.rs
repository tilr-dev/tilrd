//! Exec over the protocol (UC-077): run a program on the server's machine
//! without a shell or a pane, its stdout and stderr apart, its exit code or
//! signal, a time limit, cancelling, stdin. What the driver keeps of one
//! run; the messages are tilr.proto's (ExecStart ... ExecExit).

use std::time::Instant;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tilr_proto::{ExecExit, ExecOutput};

use crate::session::ms;
use crate::transfer::hex;

/// The HelloAck feature a server announces Exec with.
pub const FEATURE: &str = "exec";

/// Stdin a client may have sent ahead of what the program took (ExecInputAck).
pub const INPUT_WINDOW: u64 = 4 << 20;

/// Most stdin in one ExecInput the driver sends.
pub const INPUT_CHUNK: usize = 64 * 1024;

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
    /// Stdin bytes sent, and taken by the program as the server says.
    pub sent: u64,
    pub taken: u64,
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
            sent: 0,
            taken: 0,
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
            "timed_out": exit.timed_out, "cancelled": exit.cancelled, "reason": exit.reason,
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
