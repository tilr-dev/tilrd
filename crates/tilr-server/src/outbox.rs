//! The way from the supervisor to one client's connection: a bounded queue of
//! packets, and how far behind the connection's writer is with it. A writer
//! that holds a packet queued long ago is behind the screens it carries: the
//! supervisor then skips that client's frames and sends it the latest screen
//! whole once it caught up (see `Supervisor::on_pane_output`).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::mpsc;

/// Packets queued for one client before it is considered stuck and dropped.
const CLIENT_QUEUE: usize = 1024;
/// A client whose writer has held a queued packet for longer than this lags:
/// what it is sent is that much late, and a newer screen is worth more.
const LAG: Duration = Duration::from_millis(250);

/// A packet on its way to a client. `read_at`, for a pane's packet, is when
/// the supervisor read it from the pane process.
pub struct Outgoing {
    pub packet: Bytes,
    pub read_at: Option<Instant>,
}

impl From<Bytes> for Outgoing {
    fn from(packet: Bytes) -> Self {
        Self {
            packet,
            read_at: None,
        }
    }
}

/// An [`Outgoing`] with when it was queued.
struct Queued {
    outgoing: Outgoing,
    at: u64,
}

/// What the supervisor and the connection's writer share of a queue.
#[derive(Default)]
struct Backlog {
    /// Packets queued and not yet written, the one being written included.
    packets: AtomicUsize,
    /// When the oldest of them was queued, in microseconds of [`now`].
    head_at: AtomicU64,
    /// The supervisor wants to hear when the queue is empty again.
    want_drained: AtomicBool,
}

/// Where the supervisor queues packets for one client; clones go to the
/// tasks that send it things of their own (the agent's streams).
#[derive(Clone)]
pub struct ToClient {
    tx: mpsc::Sender<Queued>,
    backlog: Arc<Backlog>,
}

/// The writer's end of a client's queue.
pub struct Outbox {
    rx: mpsc::Receiver<Queued>,
    backlog: Arc<Backlog>,
}

pub fn channel() -> (ToClient, Outbox) {
    let (tx, rx) = mpsc::channel(CLIENT_QUEUE);
    let backlog = Arc::new(Backlog::default());
    (
        ToClient {
            tx,
            backlog: Arc::clone(&backlog),
        },
        Outbox { rx, backlog },
    )
}

/// Microseconds since the first call: an instant that fits an atomic.
fn now() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    u64::try_from(BASE.get_or_init(Instant::now).elapsed().as_micros()).unwrap_or(u64::MAX)
}

impl ToClient {
    /// Queue a packet, unless the queue is full (the client is stuck).
    pub fn try_send(&self, outgoing: Outgoing) -> Result<(), Outgoing> {
        let at = self.queue_one();
        self.tx.try_send(Queued { outgoing, at }).map_err(|err| {
            self.backlog.packets.fetch_sub(1, Ordering::Relaxed);
            match err {
                mpsc::error::TrySendError::Full(queued)
                | mpsc::error::TrySendError::Closed(queued) => queued.outgoing,
            }
        })
    }

    /// Queue a packet, waiting for room. `Err` once the connection is gone.
    pub async fn send(&self, outgoing: Outgoing) -> Result<(), Outgoing> {
        let at = self.queue_one();
        self.tx.send(Queued { outgoing, at }).await.map_err(|err| {
            self.backlog.packets.fetch_sub(1, Ordering::Relaxed);
            err.0.outgoing
        })
    }

    /// Count a packet in; an empty queue starts its wait now.
    fn queue_one(&self) -> u64 {
        let at = now();
        if self.backlog.packets.fetch_add(1, Ordering::Relaxed) == 0 {
            self.backlog.head_at.store(at, Ordering::Relaxed);
        }
        at
    }

    /// Whether what is queued for the client waits for longer than [`LAG`].
    pub fn lagging(&self) -> bool {
        self.backlog.packets.load(Ordering::Relaxed) > 0
            && now().saturating_sub(self.backlog.head_at.load(Ordering::Relaxed))
                > LAG.as_micros() as u64
    }

    /// Whether the connection's writer is gone.
    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Whether everything queued is written out.
    pub fn drained(&self) -> bool {
        self.backlog.packets.load(Ordering::Relaxed) == 0
    }

    /// Have the writer say so (see [`Outbox::written`]) the next time the
    /// queue is empty.
    pub fn want_drained(&self) {
        self.backlog.want_drained.store(true, Ordering::Relaxed);
    }
}

impl Outbox {
    /// The next packet to write; `None` once every sender is gone.
    pub async fn recv(&mut self) -> Option<Outgoing> {
        let queued = self.rx.recv().await?;
        self.backlog.head_at.store(queued.at, Ordering::Relaxed);
        Some(queued.outgoing)
    }

    /// The next packet if one is queued, without waiting.
    pub fn try_recv(&mut self) -> Option<Outgoing> {
        let queued = self.rx.try_recv().ok()?;
        self.backlog.head_at.store(queued.at, Ordering::Relaxed);
        Some(queued.outgoing)
    }

    /// The packet [`recv`](Self::recv) gave is written. `true` if that
    /// emptied the queue and the supervisor asked to hear of it.
    pub fn written(&self) -> bool {
        self.backlog.packets.fetch_sub(1, Ordering::Relaxed) == 1
            && self.backlog.want_drained.swap(false, Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_packet_held_too_long_is_lag_and_an_empty_queue_is_reported_once() {
        let (tx, mut outbox) = channel();
        assert!(!tx.lagging());
        assert!(tx.try_send(Bytes::from("a").into()).is_ok());
        assert!(tx.try_send(Bytes::from("b").into()).is_ok());
        assert!(!tx.lagging(), "just queued");
        tx.want_drained();
        let a = outbox.recv().await.unwrap();
        assert_eq!(a.packet, "a");
        tokio::time::sleep(LAG + Duration::from_millis(50)).await;
        assert!(tx.lagging(), "held past the limit");
        assert!(!outbox.written(), "one more to go");
        outbox.recv().await.unwrap();
        assert!(outbox.written(), "empty, and asked about");
        assert!(tx.drained());
        assert!(!tx.lagging());
        assert!(tx.try_send(Bytes::from("c").into()).is_ok());
        outbox.recv().await.unwrap();
        assert!(!outbox.written(), "said once per ask");
    }
}
