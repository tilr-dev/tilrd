//! Timing as the client sees it: every frame (when it came, how big, what
//! the pane says it cost) and the latency of each input, from sending it to
//! the first frame that carries its seq (Frame.last_input_seq).

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use serde_json::{Value, json};
use tilr_proto::Frame;

use crate::now_ms;
use crate::session::ms;

/// Frames kept for `frames`; the oldest go first.
const KEEP_FRAMES: usize = 20_000;
/// Seqs of one connection lie in [seq_base, seq_base + 2^32).
const SEQ_SPAN: u64 = 1 << 32;

#[derive(Default)]
pub struct Trace {
    seq_base: u64,
    /// Inputs of this connection not yet seen in a frame, by seq.
    sent: BTreeMap<u64, Instant>,
    latencies_ms: Vec<f64>,
    /// Inputs that never showed in a frame before their connection went.
    unanswered: u64,
    frames: VecDeque<Value>,
    frames_seen: u64,
    frame_bytes: u64,
    last_frame: Option<Instant>,
    max_gap_ms: f64,
}

impl Trace {
    pub fn new_connection(&mut self, seq_base: u64) {
        self.unanswered += self.sent.len() as u64;
        self.sent.clear();
        self.seq_base = seq_base;
    }

    pub fn input_sent(&mut self, seq: u64) {
        self.sent.insert(seq, Instant::now());
    }

    pub fn frame(&mut self, frame: &Frame, bytes: usize) {
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            self.max_gap_ms = self.max_gap_ms.max(ms(now - last));
        }
        self.last_frame = Some(now);
        let seq = frame.last_input_seq;
        if seq >= self.seq_base && seq < self.seq_base + SEQ_SPAN {
            // A frame carrying one of our seqs answers every earlier one too.
            let answered: Vec<u64> = self.sent.range(..=seq).map(|(&s, _)| s).collect();
            for s in answered {
                let sent = self.sent.remove(&s).unwrap_or(now);
                self.latencies_ms.push(ms(now - sent));
            }
        }
        let timing = frame.timing.unwrap_or_default();
        self.frames_seen += 1;
        self.frame_bytes += bytes as u64;
        if self.frames.len() == KEEP_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(json!({
            "i": self.frames_seen, "t": now_ms(), "pane": frame.pane_id, "seq": frame.seq,
            "full": frame.full, "rows": frame.changed.len(), "bytes": bytes,
            "input_seq": seq, "wait_us": timing.wait_us, "parse_us": timing.parse_us,
            "build_us": timing.build_us, "encode_us": timing.encode_us,
            "input_us": timing.input_us, "output_bytes": timing.output_bytes,
        }));
    }

    /// Input latency so far: percentiles in ms; `reset` starts over.
    pub fn latency(&mut self, reset: bool) -> Value {
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let pct = |p: f64| -> Value {
            if sorted.is_empty() {
                return Value::Null;
            }
            let at = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
            json!(sorted[at])
        };
        let out = json!({
            "count": sorted.len(), "pending": self.sent.len(), "unanswered": self.unanswered,
            "p50_ms": pct(50.0), "p90_ms": pct(90.0), "p99_ms": pct(99.0),
            "max_ms": sorted.last(), "min_ms": sorted.first(),
            "mean_ms": if sorted.is_empty() { Value::Null } else {
                json!((sorted.iter().sum::<f64>() / sorted.len() as f64 * 10.0).round() / 10.0)
            },
        });
        if reset {
            self.latencies_ms.clear();
            self.unanswered = 0;
        }
        out
    }

    /// Frames with an index after `since` (of `pane` only, if given), and totals.
    pub fn frames(&mut self, since: u64, pane: Option<&str>, reset: bool) -> Value {
        let list: Vec<&Value> = self
            .frames
            .iter()
            .filter(|f| f["i"].as_u64().unwrap_or(0) > since)
            .filter(|f| pane.is_none_or(|p| f["pane"] == p))
            .collect();
        let out = json!({
            "frames": list, "seen": self.frames_seen, "bytes": self.frame_bytes,
            "max_gap_ms": self.max_gap_ms,
        });
        if reset {
            self.max_gap_ms = 0.0;
        }
        out
    }
}
