//! The way from the supervisor to one client's connection, in two lanes,
//! and how far behind the connection's writer is with it.
//!
//! The *interactive* lane carries what the user waits for: screen frames,
//! answers, acknowledgements, the pane list. It is a bounded queue of
//! packets; a client that lets it fill up is stuck and dropped. A writer that
//! holds a packet queued long ago is behind the screens it carries: the
//! supervisor then skips that client's frames and sends it the latest screen
//! whole once it caught up (see `Supervisor::on_pane_output`).
//!
//! The *bulk* lane carries data in quantity: file content (`FsData`) and the
//! bytes of streams (`StreamData`, and the `StreamClose` that must come after
//! them). It is bounded in bytes, [`BULK_BYTES`] per client: whoever produces
//! bulk data waits for room ([`ToClient::send_bulk`]), so a slow client holds
//! the file read or the program back rather than the server's memory
//! growing, and is never dropped for it.
//!
//! The writer takes from the interactive lane first and from the bulk lane
//! only when the interactive one is empty: a frame waits behind at most the
//! one bulk packet being written, never behind megabytes of file.
//!
//! Nor behind megabytes the network holds: what the server has written, the
//! client's kernel may still hold unread (a receive buffer grows to several
//! MB), and a frame written after it waits for the client to read through
//! all of it. So bulk data is also held to a window of
//! [`BULK_WINDOW`] bytes written but not yet read by the client, known by
//! WebSocket pings ([`BulkWindow`]): every [`BULK_MARK`] bytes of bulk the
//! writer sends a ping carrying how far it is, and the client's pong, which
//! comes once the client has read up to it, moves the window on. Its
//! liveness pings carry the position too, so a pong to any ping does.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

/// Packets queued for one client in the interactive lane before it is
/// considered stuck and dropped.
const CLIENT_QUEUE: usize = 1024;
/// Bytes of bulk packets queued for one client at most; whoever has more to
/// send waits until the writer has written some.
pub const BULK_BYTES: usize = 4 << 20;
/// Bulk bytes written to a client that it has not yet read (by its pongs)
/// before the writer waits.
pub const BULK_WINDOW: u64 = 1 << 20;
/// Bulk bytes between the pings that measure the window.
const BULK_MARK: u64 = 128 << 10;
/// What a ping that marks bulk data starts with; the rest is the position.
const MARK: &[u8] = b"tilr-bulk:";
/// A client whose writer has held a queued packet for longer than this lags:
/// what it is sent is that much late, and a newer screen is worth more.
const LAG: Duration = Duration::from_millis(250);

/// A packet on its way to a client. `read_at`, for a pane's packet, is when
/// the supervisor read it from the pane process.
pub struct Outgoing {
    pub packet: Bytes,
    pub read_at: Option<Instant>,
    /// For a bulk packet: its room in the bulk lane, given back when it is
    /// written (dropped); `None` for one queued without room
    /// ([`ToClient::queue_bulk_now`]).
    credit: Option<OwnedSemaphorePermit>,
    /// Whether it came by the bulk lane.
    bulk: bool,
}

impl From<Bytes> for Outgoing {
    fn from(packet: Bytes) -> Self {
        Self {
            packet,
            read_at: None,
            credit: None,
            bulk: false,
        }
    }
}

impl Outgoing {
    pub fn new(packet: Bytes, read_at: Option<Instant>) -> Self {
        Self {
            packet,
            read_at,
            credit: None,
            bulk: false,
        }
    }

    /// Whether it came by the bulk lane.
    pub fn bulk(&self) -> bool {
        self.bulk
    }
}

/// An interactive [`Outgoing`] with when it was queued.
struct Queued {
    outgoing: Outgoing,
    at: u64,
}

/// What the supervisor and the connection's writer share of the
/// interactive lane.
#[derive(Default)]
struct Backlog {
    /// Packets queued and not yet written, the one being written included.
    packets: AtomicUsize,
    /// When the oldest of them was queued, in microseconds of [`now`].
    head_at: AtomicU64,
    /// The supervisor wants to hear when the queue is empty again.
    want_drained: AtomicBool,
    /// Bulk packets queued without room ([`ToClient::queue_bulk_now`]) and
    /// not yet written.
    bulk_now: AtomicUsize,
}

/// Where the supervisor queues packets for one client; clones go to the
/// tasks that send it things of their own (file reads, the agent's
/// streams).
#[derive(Clone)]
pub struct ToClient {
    tx: mpsc::Sender<Queued>,
    bulk_tx: mpsc::UnboundedSender<Outgoing>,
    /// Room in the bulk lane, in bytes.
    credit: Arc<Semaphore>,
    backlog: Arc<Backlog>,
}

/// The writer's end of a client's lanes.
pub struct Outbox {
    rx: mpsc::Receiver<Queued>,
    bulk_rx: mpsc::UnboundedReceiver<Outgoing>,
    credit: Arc<Semaphore>,
    backlog: Arc<Backlog>,
}

pub fn channel() -> (ToClient, Outbox) {
    let (tx, rx) = mpsc::channel(CLIENT_QUEUE);
    let (bulk_tx, bulk_rx) = mpsc::unbounded_channel();
    let credit = Arc::new(Semaphore::new(BULK_BYTES));
    let backlog = Arc::new(Backlog::default());
    (
        ToClient {
            tx,
            bulk_tx,
            credit: Arc::clone(&credit),
            backlog: Arc::clone(&backlog),
        },
        Outbox {
            rx,
            bulk_rx,
            credit,
            backlog,
        },
    )
}

/// Microseconds since the first call: an instant that fits an atomic.
fn now() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    u64::try_from(BASE.get_or_init(Instant::now).elapsed().as_micros()).unwrap_or(u64::MAX)
}

impl ToClient {
    /// Queue a packet in the interactive lane, unless it is full (the client
    /// is stuck).
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

    /// Queue a packet in the bulk lane, waiting for room: at most
    /// [`BULK_BYTES`] are queued (a larger packet waits for the lane to be
    /// empty). `Err` once the connection is gone.
    pub async fn send_bulk(&self, packet: Bytes) -> Result<(), Bytes> {
        let bytes = u32::try_from(packet.len().clamp(1, BULK_BYTES)).unwrap_or(u32::MAX);
        let Ok(credit) = Arc::clone(&self.credit).acquire_many_owned(bytes).await else {
            return Err(packet);
        };
        self.bulk_tx
            .send(Outgoing {
                packet,
                read_at: None,
                credit: Some(credit),
                bulk: true,
            })
            .map_err(|err| err.0.packet)
    }

    /// Queue a small packet in the bulk lane at once, behind the bulk
    /// packets queued before it, without waiting for room: the last word of
    /// something whose sender cannot wait (an exec given up at a reload).
    /// `Err` once the connection is gone.
    pub fn queue_bulk_now(&self, packet: Bytes) -> Result<(), Bytes> {
        self.backlog.bulk_now.fetch_add(1, Ordering::Relaxed);
        self.bulk_tx
            .send(Outgoing {
                packet,
                read_at: None,
                credit: None,
                bulk: true,
            })
            .map_err(|err| {
                self.backlog.bulk_now.fetch_sub(1, Ordering::Relaxed);
                err.0.packet
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

    /// Whether what is queued for the client in the interactive lane waits
    /// for longer than [`LAG`].
    pub fn lagging(&self) -> bool {
        self.backlog.packets.load(Ordering::Relaxed) > 0
            && now().saturating_sub(self.backlog.head_at.load(Ordering::Relaxed))
                > LAG.as_micros() as u64
    }

    /// Whether the connection's writer is gone.
    pub fn closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Whether everything queued in the interactive lane is written out.
    pub fn drained(&self) -> bool {
        self.backlog.packets.load(Ordering::Relaxed) == 0
    }

    /// Whether nothing is queued in either lane, nor being written.
    pub fn idle(&self) -> bool {
        self.drained()
            && self.credit.available_permits() == BULK_BYTES
            && self.backlog.bulk_now.load(Ordering::Relaxed) == 0
    }

    /// Have the writer say so (see [`Outbox::written`]) the next time the
    /// interactive lane is empty.
    pub fn want_drained(&self) {
        self.backlog.want_drained.store(true, Ordering::Relaxed);
    }
}

impl Outbox {
    /// The next packet to write, interactive ones first, bulk ones only if
    /// `bulk` (the window is open); `None` once every sender is gone and the
    /// lanes it may take from are empty.
    pub async fn recv(&mut self, bulk: bool) -> Option<Outgoing> {
        let queued = tokio::select! {
            biased;
            Some(queued) = self.rx.recv() => queued,
            Some(bulk) = self.bulk_rx.recv(), if bulk => return Some(bulk),
            else => return None,
        };
        self.backlog.head_at.store(queued.at, Ordering::Relaxed);
        Some(queued.outgoing)
    }

    /// The next packet if one is queued, without waiting.
    pub fn try_recv(&mut self) -> Option<Outgoing> {
        match self.rx.try_recv() {
            Ok(queued) => {
                self.backlog.head_at.store(queued.at, Ordering::Relaxed);
                Some(queued.outgoing)
            }
            Err(_) => self.bulk_rx.try_recv().ok(),
        }
    }

    /// The packet [`recv`](Self::recv) gave is written. `true` if that
    /// emptied the interactive lane and the supervisor asked to hear of it.
    /// A bulk packet gives its room back.
    pub fn written(&self, outgoing: Outgoing) -> bool {
        if outgoing.bulk() {
            if outgoing.credit.is_none() {
                self.backlog.bulk_now.fetch_sub(1, Ordering::Relaxed);
            }
            return false;
        }
        self.backlog.packets.fetch_sub(1, Ordering::Relaxed) == 1
            && self.backlog.want_drained.swap(false, Ordering::Relaxed)
    }
}

/// How much bulk data a client has been written and has read, by the pongs
/// to the writer's marking pings. The writer keeps it; the connection's
/// reader, which sees the pongs, shares [`BulkAcks`].
pub struct BulkWindow {
    /// Bulk bytes written.
    sent: u64,
    /// Where the last marking ping was.
    marked: u64,
    acks: BulkAcks,
}

/// The read side of a [`BulkWindow`]: how far the client has read.
#[derive(Clone, Default)]
pub struct BulkAcks {
    read: Arc<AtomicU64>,
    /// Bulk bytes written, as the writer last said: no pong acknowledges
    /// more.
    sent: Arc<AtomicU64>,
    moved: Arc<tokio::sync::Notify>,
}

impl BulkWindow {
    pub fn new() -> (Self, BulkAcks) {
        let acks = BulkAcks::default();
        (
            Self {
                sent: 0,
                marked: 0,
                acks: acks.clone(),
            },
            acks,
        )
    }

    /// Whether more bulk data may be written now.
    pub fn open(&self) -> bool {
        self.sent
            .saturating_sub(self.acks.read.load(Ordering::Relaxed))
            < BULK_WINDOW
    }

    /// Until the client has read more.
    pub async fn moved(&self) {
        self.acks.moved.notified().await;
    }

    /// `bytes` of bulk were written. Returns the payload of a ping to send
    /// now, if it is time for a mark (and always once the window is full,
    /// so that its pong opens it again).
    pub fn wrote(&mut self, bytes: usize) -> Option<Bytes> {
        self.sent += bytes as u64;
        self.acks.sent.store(self.sent, Ordering::Relaxed);
        if self.sent - self.marked < BULK_MARK && self.open() {
            return None;
        }
        self.marked = self.sent;
        Some(self.position())
    }

    /// The payload of a ping that marks the bulk data written so far. Every
    /// ping the writer sends carries it, the liveness ones too: a client may
    /// answer only the latest of several pings (RFC 6455 allows it), and a
    /// mark it never answers must not keep the window closed.
    pub fn position(&self) -> Bytes {
        let mut payload = MARK.to_vec();
        payload.extend_from_slice(&self.sent.to_be_bytes());
        payload.into()
    }
}

impl BulkAcks {
    /// A pong from the client: if it answers a marking ping, the client has
    /// read that far.
    pub fn pong(&self, payload: &[u8]) {
        let Some(position) = payload
            .strip_prefix(MARK)
            .and_then(|rest| <[u8; 8]>::try_from(rest).ok())
        else {
            return;
        };
        // A pong is read after the data it marks was written, so an honest
        // one is never ahead of `sent`; one that is (a client that lies or
        // errs) acknowledges everything written so far, not more.
        let position = u64::from_be_bytes(position).min(self.sent.load(Ordering::Relaxed));
        self.read.fetch_max(position, Ordering::Relaxed);
        self.moved.notify_one();
    }
}

impl Drop for Outbox {
    /// Whoever waits for room in the bulk lane learns the connection is gone.
    fn drop(&mut self) {
        self.credit.close();
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
        let a = outbox.recv(true).await.unwrap();
        assert_eq!(a.packet, "a");
        tokio::time::sleep(LAG + Duration::from_millis(50)).await;
        assert!(tx.lagging(), "held past the limit");
        assert!(!outbox.written(a), "one more to go");
        let b = outbox.recv(true).await.unwrap();
        assert!(outbox.written(b), "empty, and asked about");
        assert!(tx.drained());
        assert!(!tx.lagging());
        assert!(tx.try_send(Bytes::from("c").into()).is_ok());
        let c = outbox.recv(true).await.unwrap();
        assert!(!outbox.written(c), "said once per ask");
    }

    #[tokio::test]
    async fn interactive_packets_go_ahead_of_bulk_ones() {
        let (tx, mut outbox) = channel();
        tx.send_bulk(Bytes::from("file 1")).await.unwrap();
        tx.send_bulk(Bytes::from("file 2")).await.unwrap();
        assert!(tx.try_send(Bytes::from("frame").into()).is_ok());
        let order: Vec<Bytes> = [(); 3]
            .iter()
            .map(|()| outbox.try_recv().unwrap().packet)
            .collect();
        assert_eq!(order, ["frame", "file 1", "file 2"]);
        assert!(tx.try_send(Bytes::from("ack").into()).is_ok());
        tx.send_bulk(Bytes::from("file 3")).await.unwrap();
        assert_eq!(outbox.recv(true).await.unwrap().packet, "ack");
        let bulk = outbox.recv(true).await.unwrap();
        assert!(bulk.bulk());
        assert!(!outbox.written(bulk), "bulk is not what lags");
    }

    #[test]
    fn the_bulk_window_closes_until_the_client_has_read_its_marks() {
        assert!(
            BulkWindow::new().0.wrote(1000).is_none(),
            "not yet a mark's worth"
        );
        let (mut window, acks) = BulkWindow::new();
        assert!(window.open());
        let mut pings = Vec::new();
        while window.open() {
            pings.extend(window.wrote(64 << 10));
        }
        assert_eq!(window.sent, BULK_WINDOW);
        assert_eq!(pings.len() as u64, BULK_WINDOW / BULK_MARK, "{pings:?}");
        acks.pong(b"not ours");
        assert!(!window.open(), "only marking pongs count");
        acks.pong(&pings[0]);
        assert!(window.open(), "the client read up to the first mark");
        acks.pong(&pings[pings.len() - 1]);
        acks.pong(&pings[1]);
        assert_eq!(acks.read.load(Ordering::Relaxed), window.sent, "never back");
    }

    #[test]
    fn any_pong_moves_the_window() {
        let (mut window, acks) = BulkWindow::new();
        let mut marks = 0;
        while window.open() {
            marks += usize::from(window.wrote(64 << 10).is_some());
        }
        assert!(marks > 0);
        // The client answers none of the marks, only a later liveness ping.
        acks.pong(&window.position());
        assert!(
            window.open(),
            "the liveness ping's pong acknowledged it all"
        );
        assert_eq!(acks.read.load(Ordering::Relaxed), window.sent);
    }

    #[test]
    fn a_pong_beyond_what_was_sent_opens_no_more_than_the_window() {
        let (mut window, acks) = BulkWindow::new();
        window.wrote(1000);
        let mut ahead = MARK.to_vec();
        ahead.extend_from_slice(&(100 * BULK_WINDOW).to_be_bytes());
        acks.pong(&ahead);
        let mut written = 0;
        while window.open() {
            window.wrote(64 << 10);
            written += 64 << 10;
        }
        assert_eq!(
            written, BULK_WINDOW,
            "the window, not what the pong claimed"
        );
    }

    #[tokio::test]
    async fn a_packet_queued_now_goes_after_the_bulk_before_it() {
        let (tx, mut outbox) = channel();
        // The lane is full: a sender would wait for room.
        tx.send_bulk(Bytes::from(vec![0; BULK_BYTES]))
            .await
            .unwrap();
        tx.queue_bulk_now(Bytes::from("the end")).unwrap();
        assert!(!tx.idle());
        let first = outbox.recv(true).await.unwrap();
        assert_eq!(first.packet.len(), BULK_BYTES);
        assert!(!outbox.written(first));
        assert!(!tx.idle(), "the last one is still queued");
        let last = outbox.recv(true).await.unwrap();
        assert!(last.bulk());
        assert_eq!(last.packet, "the end");
        assert!(!outbox.written(last));
        assert!(tx.idle());
        drop(outbox);
        assert!(tx.queue_bulk_now(Bytes::from("late")).is_err());
    }

    #[tokio::test]
    async fn bulk_waits_for_room_counted_in_bytes() {
        let (tx, mut outbox) = channel();
        let half = Bytes::from(vec![0; BULK_BYTES / 2]);
        tx.send_bulk(half.clone()).await.unwrap();
        tx.send_bulk(half.clone()).await.unwrap();
        let third = tokio::spawn({
            let tx = tx.clone();
            async move { tx.send_bulk(Bytes::from("more")).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!third.is_finished(), "the lane is full: the sender waits");
        let first = outbox.recv(true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!third.is_finished(), "taken, but not yet written");
        outbox.written(first);
        third.await.unwrap().unwrap();
        drop(outbox);
        assert!(
            tx.send_bulk(Bytes::from("late")).await.is_err(),
            "the connection is gone"
        );
    }
}
