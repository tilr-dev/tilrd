//! Helpers of `bus-core-panes` for the bus tests, beside `bus.rs`: `pane.*` calls
//! as the owner, a pane's screen from the chunks of `pane.output`, waiting for text.
//!
//! Owner: `bus-core-panes`.

#![allow(
    dead_code,
    reason = "each test binary that declares `mod common` uses some of these"
)]

use std::time::Duration;

use bytes::Bytes;
use prost::Message;
use tilr_bus::pane::{
    CreateRequest, Frame, InputAck, InputRequest, ListReply, ListRequest, PaneInfo, PaneOutput,
    RawInput, ReadReply, ReadRequest, input_request::Input, pane_output::Output,
};
use tilr_bus::{Bus, BusError, CallOptions, ChunkStream, StreamOptions};

use super::TIMEOUT;

/// The command of a test pane: a plain `sh` with the prompt `$ `.
pub fn shell() -> Vec<String> {
    ["/bin/sh", "-c", "PS1='$ ' exec /bin/sh"]
        .map(String::from)
        .to_vec()
}

/// Call a method of `pane.*` with protobuf messages.
pub async fn call<Req: Message, Resp: Message + Default>(
    bus: &Bus,
    method: &str,
    request: &Req,
) -> Result<Resp, BusError> {
    bus.call_typed(method, request, CallOptions::default())
        .await
}

/// `pane.create` of a pane running `command`.
pub async fn create(bus: &Bus, cols: u32, rows: u32, command: Vec<String>) -> PaneInfo {
    call(
        bus,
        "pane.create",
        &CreateRequest {
            cols,
            rows,
            command,
            ..Default::default()
        },
    )
    .await
    .expect("pane.create")
}

/// A shell pane of 100x30, once its prompt shows.
pub async fn create_shell(bus: &Bus) -> PaneInfo {
    let info = create(bus, 100, 30, shell()).await;
    wait_text(bus, &info.pane_id, |lines| {
        lines.first().is_some_and(|line| line == "$")
    })
    .await;
    info
}

pub async fn list(bus: &Bus) -> Vec<PaneInfo> {
    call::<_, ListReply>(bus, "pane.list", &ListRequest {})
        .await
        .expect("pane.list")
        .panes
}

/// `pane.input {raw}` without seqs.
pub async fn raw(bus: &Bus, pane_id: &str, data: &str) -> Result<InputAck, BusError> {
    call(
        bus,
        "pane.input",
        &InputRequest {
            pane_id: pane_id.into(),
            input: Some(Input::Raw(RawInput {
                data: Bytes::copy_from_slice(data.as_bytes()),
            })),
            ..Default::default()
        },
    )
    .await
}

pub async fn read(bus: &Bus, request: &ReadRequest) -> Result<ReadReply, BusError> {
    call(bus, "pane.read", request).await
}

/// Read the pane until `done` holds for its lines, waiting for changes.
pub async fn wait_text(bus: &Bus, pane_id: &str, done: impl Fn(&[String]) -> bool) -> ReadReply {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    let mut since = None;
    loop {
        let reply = read(
            bus,
            &ReadRequest {
                pane_id: pane_id.into(),
                since_seq: since,
                wait_ms: Some(1000),
                ..Default::default()
            },
        )
        .await
        .expect("pane.read");
        if done(&reply.lines) {
            return reply;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the pane never showed it:\n{}",
            reply.lines.join("\n")
        );
        since = Some(reply.seq);
    }
}

/// What a consumer of `pane.output` sees of a pane.
#[derive(Default)]
pub struct Screen {
    pub rows: Vec<String>,
    /// The last frame, without its rows.
    pub frame: Option<Frame>,
    pub frames: usize,
    pub full_frames: usize,
}

impl Screen {
    pub fn apply(&mut self, frame: &Frame) {
        if frame.full {
            self.rows = vec![String::new(); frame.rows as usize];
            self.full_frames += 1;
        }
        self.frames += 1;
        for row in &frame.changed {
            let text: String = row.runs.iter().map(|run| run.text.as_str()).collect();
            if let Some(slot) = self.rows.get_mut(row.y as usize) {
                *slot = text.trim_end().to_owned();
            }
        }
        self.frame = Some(Frame {
            changed: Vec::new(),
            ..frame.clone()
        });
    }

    pub fn text(&self) -> String {
        self.rows.join("\n")
    }

    /// Apply a chunk of `pane.output`; the frame, if it is one.
    pub fn apply_chunk(&mut self, payload: &[u8]) -> Option<Frame> {
        let output = PaneOutput::decode(payload).expect("a PaneOutput chunk");
        match output.output {
            Some(Output::Frame(frame)) => {
                self.apply(&frame);
                Some(*frame)
            }
            _ => None,
        }
    }
}

/// A `pane.output` stream of the owner and what it shows.
pub struct Watching {
    pub stream: ChunkStream,
    pub screen: Screen,
}

impl Watching {
    /// Open `pane.output` with a 4 MiB window, as clients do.
    pub async fn open(bus: &Bus, pane_id: &str) -> Result<Self, BusError> {
        let request = tilr_bus::pane::OutputRequest {
            pane_id: pane_id.into(),
            ..Default::default()
        };
        let stream = bus
            .stream(
                "pane.output",
                request.encode_to_vec(),
                StreamOptions::default().window(4 << 20),
            )
            .await?;
        Ok(Self {
            stream,
            screen: Screen::default(),
        })
    }

    /// The next frame; `None` at the end of the stream.
    pub async fn next_frame(&mut self) -> Option<Result<Frame, BusError>> {
        loop {
            let chunk = tokio::time::timeout(TIMEOUT, self.stream.next())
                .await
                .expect("nothing on pane.output in time")?;
            match chunk {
                Ok(chunk) => {
                    let payload = chunk.bytes().expect("chunk payload");
                    if let Some(frame) = self.screen.apply_chunk(&payload) {
                        return Some(Ok(frame));
                    }
                }
                Err(err) => return Some(Err(err)),
            }
        }
    }

    /// Wait until the screen shows `text`.
    pub async fn wait_screen(&mut self, text: &str) {
        while !self.screen.text().contains(text) {
            match self.next_frame().await {
                Some(Ok(_)) => {}
                other => panic!(
                    "pane.output ended ({other:?}); screen:\n{}",
                    self.screen.text()
                ),
            }
        }
    }
}

/// Poll `check` every 20 ms until it holds, for up to [`TIMEOUT`].
pub async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !check().await {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A raw client greeted as the owner with a `client_id`: its calls go out in
/// the order written and its replies are read one by one.
pub async fn raw_with_client_id(
    server: &super::bus::BusServer,
    client_name: &str,
    client_id: &str,
) -> super::bus::Raw {
    use tilr_bus::bus::{Hello, envelope::Msg};
    let mut raw = server.raw().await;
    raw.send(Msg::Hello(Hello {
        protocol_version: tilr_bus::PROTOCOL_VERSION,
        client_name: client_name.into(),
        client_id: client_id.into(),
        ..Hello::default()
    }))
    .await;
    match raw.recv().await {
        Some(Msg::HelloAck(ack)) => raw.ack = Some(ack),
        other => panic!("{client_name}: expected HelloAck, got {other:?}"),
    }
    raw
}

/// Write a protobuf call `id` of `method` on a raw client.
pub async fn send_call(raw: &mut super::bus::Raw, id: u64, method: &str, request: &impl Message) {
    send_call_bytes(raw, id, method, request.encode_to_vec()).await;
}

/// Write call `id` of `method` with `payload` as it is on a raw client.
pub async fn send_call_bytes(raw: &mut super::bus::Raw, id: u64, method: &str, payload: Vec<u8>) {
    use tilr_bus::bus::{Call, envelope::Msg};
    raw.send(Msg::Call(Call {
        id,
        method: method.into(),
        payload: payload.into(),
        ..Call::default()
    }))
    .await;
}

/// The next reply on a raw client: its id and its payload or error.
pub async fn next_reply(raw: &mut super::bus::Raw) -> (u64, Result<Bytes, (String, String)>) {
    use tilr_bus::bus::{envelope::Msg, reply};
    match raw.recv().await {
        Some(Msg::Reply(r)) => match r.result {
            Some(reply::Result::Payload(payload)) => (r.id, Ok(payload)),
            Some(reply::Result::Error(error)) => (r.id, Err((error.code, error.message))),
            None => panic!("a reply without a result: {r:?}"),
        },
        other => panic!("expected a reply, got {other:?}"),
    }
}
