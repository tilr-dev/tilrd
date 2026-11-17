//! Helpers of `bus-streams-events` for the bus tests, beside `bus.rs`: calls with a
//! window and a deadline, credits, error replies and the drop counter of events.
//!
//! Owner: `bus-streams-events`.

use std::time::Duration;

use bytes::Bytes;
use tilr_bus::bus::{Call, Credit, envelope::Msg, reply};

use super::bus::{BusServer, Raw};
use super::{TIMEOUT, http_get, sample};

/// A `Call` of `method` with `payload`, a stream window (0: the default) and a
/// deadline (0: none given).
pub fn call_with(id: u64, method: &str, payload: &str, window: u32, timeout_ms: u32) -> Msg {
    Msg::Call(Call {
        id,
        method: method.into(),
        payload: Bytes::copy_from_slice(payload.as_bytes()),
        window,
        timeout_ms,
        ..Call::default()
    })
}

/// `Credit` of `bytes` for the stream `id`.
pub fn credit(id: u64, bytes: usize) -> Msg {
    Msg::Credit(Credit {
        id,
        bytes: u32::try_from(bytes).unwrap(),
    })
}

/// The id, code and message of an error `Reply`.
pub fn error_of(msg: &Msg) -> Option<(u64, String, String)> {
    match msg {
        Msg::Reply(reply) => match &reply.result {
            Some(reply::Result::Error(error)) => {
                Some((reply.id, error.code.clone(), error.message.clone()))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Chunk payload bytes per stream id that reach `raw` until nothing comes for
/// `quiet`; anything but a chunk fails.
pub async fn chunks_until_quiet(raw: &mut Raw, quiet: Duration) -> Vec<(u64, usize)> {
    let mut seen: Vec<(u64, usize)> = Vec::new();
    while let Some(msg) = raw.recv_within(quiet).await {
        match msg {
            Msg::Chunk(chunk) => add(&mut seen, chunk.id, chunk.payload.len()),
            other => panic!("expected a chunk, got {other:?}"),
        }
    }
    seen
}

/// Add `bytes` to the count of `id`.
pub fn add(seen: &mut Vec<(u64, usize)>, id: u64, bytes: usize) {
    match seen.iter_mut().find(|(seen_id, _)| *seen_id == id) {
        Some((_, count)) => *count += bytes,
        None => seen.push((id, bytes)),
    }
}

/// The count of `id` in `seen`.
pub fn count(seen: &[(u64, usize)], id: u64) -> usize {
    seen.iter()
        .find(|(seen_id, _)| *seen_id == id)
        .map_or(0, |(_, count)| *count)
}

/// `tilrd_bus_events_dropped_total{subject}` from `GET /metrics`; 0 when absent.
pub async fn events_dropped(server: &BusServer, subject: &str) -> u64 {
    let url = format!("/metrics?token={}", server.token());
    let (status, text) = http_get(server, &url, &[]).await;
    assert_eq!(status, 200, "{text}");
    let series = format!("tilrd_bus_events_dropped_total{{subject=\"{subject}\"}}");
    sample(&text, &series).map_or(0, |value| value as u64)
}

/// Wait up to [`TIMEOUT`] for `pick` to take an envelope `raw` receives, failing
/// on any `skip` does not let pass.
pub async fn expect<T>(
    raw: &mut Raw,
    mut pick: impl FnMut(&Msg) -> Option<T>,
    skip: impl Fn(&Msg) -> bool,
) -> T {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Some(msg) = raw.recv_within(left).await else {
            panic!("closed or nothing in time");
        };
        if let Some(found) = pick(&msg) {
            return found;
        }
        assert!(skip(&msg), "unexpected {msg:?}");
    }
}

/// A `Chunk` of `bytes` bytes on the hop `hop`.
pub fn chunk(hop: u64, bytes: usize) -> Msg {
    Msg::Chunk(tilr_bus::bus::Chunk {
        id: hop,
        payload: Bytes::from(vec![b'c'; bytes]),
        content_encoding: String::new(),
    })
}

/// A `Chunk` of `bytes` bytes that do not compress (xorshift), on the hop `hop`.
pub fn noise_chunk(hop: u64, bytes: usize) -> Msg {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ hop;
    let payload: Vec<u8> = (0..bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    Msg::Chunk(tilr_bus::bus::Chunk {
        id: hop,
        payload: Bytes::from(payload),
        content_encoding: String::new(),
    })
}

/// As a provider holding `credit` on the hop `hop`, send `total` bytes in chunks
/// of `size`, waiting for the broker's `Credit` when it runs out. Returns the
/// bytes sent: `total`, or fewer if the broker cancelled the stream meanwhile.
pub async fn send_within_credit(
    provider: &mut Raw,
    hop: u64,
    mut credit: usize,
    total: usize,
    size: usize,
) -> usize {
    let mut sent = 0;
    while sent < total {
        let len = size.min(total - sent);
        while credit < len {
            match provider
                .wait_for(|msg| match msg {
                    Msg::Credit(credit) if credit.id == hop => Some(Some(credit.bytes as usize)),
                    Msg::Cancel(cancel) if cancel.id == hop => Some(None),
                    _ => None,
                })
                .await
            {
                Some(bytes) => credit += bytes,
                None => return sent,
            }
        }
        provider.send(chunk(hop, len)).await;
        credit -= len;
        sent += len;
    }
    sent
}

/// A `bus.ping` round trip: once it is answered, the broker has handled what
/// `raw` sent before it. Envelopes before the answer are skipped.
pub async fn ping(raw: &mut Raw, id: u64) {
    raw.send(Msg::Call(Call {
        id,
        method: "bus.ping".into(),
        ..Call::default()
    }))
    .await;
    raw.wait_for(|msg| matches!(msg, Msg::Reply(reply) if reply.id == id).then_some(()))
        .await;
}

/// `future`, failing the test if it is not done within [`TIMEOUT`]: a wait on a
/// call, a stream or an event that a regression leaves pending fails instead of
/// hanging the run.
pub async fn within<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(TIMEOUT, future)
        .await
        .expect("not done in time")
}
