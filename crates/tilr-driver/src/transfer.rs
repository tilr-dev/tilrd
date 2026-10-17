//! File transfers that run beside everything else the connection does: a
//! write goes up one chunk at a time between reads (FsWrite), a read comes
//! down in parts (FsRead with offset and length), several of each at once.
//! The terminal keeps being served meanwhile, which is what a case measures.

use std::fs::File;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tilr_proto::{FsData, FsRead, FsWrite};

use crate::session::ms;

/// Where the bytes of a write come from.
pub enum Source {
    Bytes(Vec<u8>),
    /// A file on the client's machine, read as the chunks go.
    File(File),
}

pub struct Upload {
    /// What the case calls it (`transfer`).
    pub id: u64,
    pub request_id: u64,
    transfer_id: u64,
    path: String,
    source: Source,
    pub total: u64,
    pub sent: u64,
    chunk: usize,
    pause: Duration,
    next_at: Instant,
    create: bool,
    truncate: bool,
    pub chunks: u64,
    /// Every chunk is on its way, the last one included.
    pub all_sent: bool,
    started: Instant,
    pub background: bool,
    sha: Sha256,
}

pub struct UploadSpec {
    pub path: String,
    pub source: Source,
    pub total: u64,
    pub chunk: usize,
    pub pause: Duration,
    pub create: bool,
    pub truncate: bool,
}

impl Upload {
    pub fn new(id: u64, request_id: u64, spec: UploadSpec, background: bool) -> Self {
        Self {
            id,
            request_id,
            // Distinct from the client's other transfers in progress: the
            // driver's own numbering serves.
            transfer_id: id,
            path: spec.path,
            source: spec.source,
            total: spec.total,
            sent: 0,
            chunk: spec.chunk.max(1),
            pause: spec.pause,
            next_at: Instant::now(),
            create: spec.create,
            truncate: spec.truncate,
            chunks: 0,
            all_sent: false,
            started: Instant::now(),
            background,
            sha: Sha256::new(),
        }
    }

    /// When the next chunk may go; `None` once all have.
    pub fn due(&self) -> Option<Instant> {
        (!self.all_sent).then_some(self.next_at)
    }

    /// The next chunk to send. A write of nothing is one empty chunk.
    pub fn next_chunk(&mut self) -> Result<FsWrite> {
        let want = (self.total - self.sent).min(self.chunk as u64) as usize;
        let data = match &mut self.source {
            Source::Bytes(bytes) => {
                let start = self.sent as usize;
                bytes[start..start + want].to_vec()
            }
            Source::File(file) => {
                let mut data = vec![0; want];
                file.read_exact(&mut data).context("read the local file")?;
                data
            }
        };
        let first = self.chunks == 0;
        let last = self.sent + want as u64 == self.total;
        self.sha.update(&data);
        Ok(FsWrite {
            request_id: self.request_id,
            transfer_id: self.transfer_id,
            path: if first {
                crate::names::to_request(&self.path).0
            } else {
                String::new()
            },
            path_bytes: if first {
                crate::names::to_request(&self.path).1
            } else {
                Vec::new()
            },
            total_size: if first { self.total } else { 0 },
            create: first && self.create,
            truncate: first && self.truncate,
            data: Bytes::from(data),
            last,
        })
    }

    /// The chunk `next_chunk` gave is on its way.
    pub fn sent_chunk(&mut self, len: usize, last: bool) {
        self.sent += len as u64;
        self.chunks += 1;
        self.all_sent = last;
        self.next_at = Instant::now() + self.pause;
    }

    pub fn outcome(&mut self, error: &str) -> Value {
        json!({
            "transfer": self.id, "kind": "write", "path": self.path, "error": error,
            "sent": self.sent, "size": self.total, "chunks": self.chunks,
            "sha256": hex(&std::mem::take(&mut self.sha).finalize()),
            "ms": ms(self.started.elapsed()),
        })
    }
}

/// Bytes a read keeps in memory to show as `text` when it has no file to go to.
const KEEP_TEXT: usize = 64 * 1024;

pub struct Download {
    pub id: u64,
    /// The request of the part on its way.
    pub request_id: u64,
    path: String,
    to: Option<File>,
    kept: Vec<u8>,
    start: u64,
    /// 0: to the end of the file.
    length: u64,
    /// Bytes one FsRead asks for; 0: everything in one.
    part: u64,
    /// What the current part asked for.
    asked: u64,
    part_got: u64,
    pub received: u64,
    file_size: Option<u64>,
    parts: u64,
    /// The next part is to be asked for.
    pub ask_next: bool,
    started: Instant,
    pub background: bool,
    sha: Sha256,
}

pub struct DownloadSpec {
    pub path: String,
    pub to: Option<File>,
    pub offset: u64,
    pub length: u64,
    pub part: u64,
}

/// What a part's FsData did to a download.
pub enum Progress {
    Going,
    Done(Value),
}

impl Download {
    pub fn new(id: u64, spec: DownloadSpec, background: bool) -> Self {
        Self {
            id,
            request_id: 0,
            path: spec.path,
            to: spec.to,
            kept: Vec::new(),
            start: spec.offset,
            length: spec.length,
            part: spec.part,
            asked: 0,
            part_got: 0,
            received: 0,
            file_size: None,
            parts: 0,
            ask_next: true,
            started: Instant::now(),
            background,
            sha: Sha256::new(),
        }
    }

    /// The request for the next part, under `request_id`.
    pub fn next_request(&mut self, request_id: u64) -> FsRead {
        let left = match self.length {
            0 => u64::MAX,
            length => length - self.received,
        };
        let asked = match self.part {
            0 => self.length,
            part => part.min(left),
        };
        self.request_id = request_id;
        self.asked = asked;
        self.part_got = 0;
        self.parts += 1;
        self.ask_next = false;
        FsRead {
            request_id,
            path: crate::names::to_request(&self.path).0,
            path_bytes: crate::names::to_request(&self.path).1,
            offset: self.start + self.received,
            length: asked,
        }
    }

    pub fn on_data(&mut self, data: &FsData) -> Progress {
        if !data.error.is_empty() {
            return Progress::Done(self.outcome(&data.error));
        }
        self.file_size = Some(data.file_size);
        if let Some(file) = &mut self.to {
            if let Err(err) = file.write_all(&data.data) {
                return Progress::Done(self.outcome(&format!("write the local file: {err}")));
            }
        } else if self.kept.len() < KEEP_TEXT {
            let room = KEEP_TEXT - self.kept.len();
            self.kept
                .extend_from_slice(&data.data[..data.data.len().min(room)]);
        }
        self.sha.update(&data.data);
        self.received += data.data.len() as u64;
        self.part_got += data.data.len() as u64;
        if !data.last {
            return Progress::Going;
        }
        let short = self.asked != 0 && self.part_got < self.asked;
        let whole = (self.length != 0 && self.received >= self.length)
            || self.start + self.received >= data.file_size;
        if self.part == 0 || short || whole {
            return Progress::Done(self.outcome(""));
        }
        self.ask_next = true;
        Progress::Going
    }

    pub fn outcome(&mut self, error: &str) -> Value {
        if let Some(file) = &mut self.to {
            let _ = file.flush();
        }
        let mut out = json!({
            "transfer": self.id, "kind": "read", "path": self.path, "error": error,
            "size": self.received, "file_size": self.file_size, "parts": self.parts,
            "sha256": hex(&std::mem::take(&mut self.sha).finalize()),
            "ms": ms(self.started.elapsed()),
        });
        if self.to.is_none() {
            out["text"] = json!(String::from_utf8_lossy(&self.kept));
        }
        out
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
