//! `GET /metrics`: what the supervisor measures, in the Prometheus text
//! exposition format (0.0.4), on the server's own port and behind the same
//! access token as `/socks`.
//!
//! Counters per pane live in the supervisor's actor, as plain numbers it
//! updates while routing; a scrape asks the actor for a [`Snapshot`]. The
//! histograms are filled by the tasks that move packets, so they are shared
//! ([`Shared`]) and atomic. CPU and memory of the processes are read only
//! when scraped.

use std::fmt::{Display, Write as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::handshake::server::Request;
use tokio_tungstenite::tungstenite::http::StatusCode;

pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// How long a client has to send its request head, and how long it may be.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST: usize = 16 * 1024;
/// How long to wait for the rest of a request line that starts like ours.
const PEEK_TIMEOUT: Duration = Duration::from_secs(1);
/// How long writing the response may take.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bounds of the histogram buckets: nanoseconds, and as written in `le`.
const BUCKETS: [(u64, &str); 11] = [
    (50_000, "0.00005"),
    (100_000, "0.0001"),
    (250_000, "0.00025"),
    (500_000, "0.0005"),
    (1_000_000, "0.001"),
    (2_500_000, "0.0025"),
    (5_000_000, "0.005"),
    (10_000_000, "0.01"),
    (25_000_000, "0.025"),
    (50_000_000, "0.05"),
    (100_000_000, "0.1"),
];

/// A histogram of durations that any task can add to.
#[derive(Default)]
pub struct Histogram {
    /// Observations per bucket, not cumulative; the last one is `+Inf`.
    counts: [AtomicU64; BUCKETS.len() + 1],
    sum_ns: AtomicU64,
}

impl Histogram {
    pub fn observe(&self, duration: Duration) {
        let ns = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        let bucket = BUCKETS
            .iter()
            .position(|&(bound, _)| ns <= bound)
            .unwrap_or(BUCKETS.len());
        self.counts[bucket].fetch_add(1, Ordering::Relaxed);
        self.sum_ns.fetch_add(ns, Ordering::Relaxed);
    }
}

/// The measurements the routing tasks make.
#[derive(Default)]
pub struct Shared {
    /// A pane's packet: read from the pane process to written to a client.
    pub route: Histogram,
    /// A client's input: read from the client to written to the pane process.
    pub route_input: Histogram,
    /// How late the supervisor's actor takes its once-a-second tick.
    pub loop_lag: Histogram,
    /// Clients the server let go, by [`ClientDrop`].
    pub client_drops: [AtomicU64; ClientDrop::ALL.len()],
}

/// Why the server let a client go, for `tilr_client_drops_total`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClientDrop {
    /// A newer connection of the same client took over.
    Superseded,
    /// Its queue was full: it did not take what it was sent.
    TooSlow,
    /// Nothing heard from it for the client timeout.
    Silent,
    /// It still sent, but did not answer the server's pings for the client
    /// timeout: the way towards it is gone.
    NoPong,
}

impl ClientDrop {
    pub const ALL: [Self; 4] = [Self::Superseded, Self::TooSlow, Self::Silent, Self::NoPong];

    pub fn label(self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::TooSlow => "too_slow",
            Self::Silent => "silent",
            Self::NoPong => "no_pong",
        }
    }
}

impl Shared {
    pub fn count_drop(&self, why: ClientDrop) {
        self.client_drops[why as usize].fetch_add(1, Ordering::Relaxed);
    }
}

/// What the supervisor counts for one pane while it routes.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct PaneCounters {
    /// Frames the pane process sent, and their bytes as routed (once per
    /// frame, whatever the number of clients).
    pub frames: u64,
    pub frame_bytes: u64,
    /// Frames handed to clients: one per frame and client.
    pub frames_sent: u64,
    /// Frames not sent to a client that lagged, which got a whole screen
    /// later instead: one per frame and client.
    pub frames_skipped: u64,
    /// Input packets (key, mouse, paste, raw, scroll) passed to the pane,
    /// and their bytes.
    pub input: u64,
    pub input_bytes: u64,
}

pub struct PaneSnapshot {
    pub id: String,
    /// Of the pane process.
    pub pid: u32,
    pub counters: PaneCounters,
}

/// What the actor knows, taken at a scrape.
pub struct Snapshot {
    pub version: String,
    pub server_id: String,
    pub clients: usize,
    pub panes: Vec<PaneSnapshot>,
}

/// CPU time and memory of a process.
#[derive(Clone, Copy, Debug)]
pub struct Usage {
    pub cpu_seconds: f64,
    /// Resident memory; on macOS the physical footprint.
    pub memory_bytes: u64,
}

/// The metrics, as the text a scrape gets. `usage` tells about a process,
/// the supervisor's and each pane's.
pub fn render(
    snapshot: &Snapshot,
    shared: &Shared,
    supervisor_pid: u32,
    usage: impl Fn(u32) -> Option<Usage>,
) -> String {
    let mut out = Exposition::default();
    out.family(
        "tilr_server_info",
        "gauge",
        "Version and id of this server.",
    );
    out.sample(
        "tilr_server_info",
        &[
            ("version", &snapshot.version),
            ("server_id", &snapshot.server_id),
        ],
        1,
    );
    out.family("tilr_server_clients", "gauge", "Clients connected.");
    out.sample("tilr_server_clients", &[], snapshot.clients);
    out.family("tilr_server_panes", "gauge", "Panes running.");
    out.sample("tilr_server_panes", &[], snapshot.panes.len());

    type Counter = fn(&PaneCounters) -> u64;
    let per_pane: [(&str, &str, Counter); 6] = [
        ("tilr_pane_frames_total", "Frames the pane produced.", |c| {
            c.frames
        }),
        (
            "tilr_pane_frame_bytes_total",
            "Bytes of the frames the pane produced, as routed (once per frame).",
            |c| c.frame_bytes,
        ),
        (
            "tilr_pane_frames_sent_total",
            "Frames handed to clients, one per frame and client.",
            |c| c.frames_sent,
        ),
        (
            "tilr_pane_frames_skipped_total",
            "Frames not sent to a client that lagged (it got the whole screen later), one per frame and client.",
            |c| c.frames_skipped,
        ),
        (
            "tilr_pane_input_total",
            "Input packets (key, mouse, paste, raw, scroll) passed to the pane.",
            |c| c.input,
        ),
        (
            "tilr_pane_input_bytes_total",
            "Bytes of the input packets passed to the pane.",
            |c| c.input_bytes,
        ),
    ];
    for (name, help, value) in per_pane {
        out.family(name, "counter", help);
        for pane in &snapshot.panes {
            out.sample(name, &[("pane", &pane.id)], value(&pane.counters));
        }
    }

    out.family(
        "tilr_client_drops_total",
        "counter",
        "Clients the server let go: superseded by a newer connection of the same client, too slow, silent, or not answering pings.",
    );
    for why in ClientDrop::ALL {
        out.sample(
            "tilr_client_drops_total",
            &[("reason", why.label())],
            shared.client_drops[why as usize].load(Ordering::Relaxed),
        );
    }

    out.histogram(
        "tilr_route_seconds",
        "From the supervisor reading a pane's packet to its WebSocket write to a client returning.",
        &shared.route,
    );
    out.histogram(
        "tilr_route_input_seconds",
        "From the supervisor reading a client's input packet to its write to the pane process returning.",
        &shared.route_input,
    );
    out.histogram(
        "tilr_supervisor_loop_lag_seconds",
        "How late the supervisor's event loop handles its once-a-second tick.",
        &shared.loop_lag,
    );

    let supervisor_usage = usage(supervisor_pid);
    let supervisor_pid = supervisor_pid.to_string();
    let mut processes = vec![(
        supervisor_usage,
        vec![("role", "supervisor"), ("pid", supervisor_pid.as_str())],
    )];
    let pane_pids: Vec<String> = snapshot.panes.iter().map(|p| p.pid.to_string()).collect();
    for (pane, pid) in snapshot.panes.iter().zip(&pane_pids) {
        processes.push((
            usage(pane.pid),
            vec![("role", "pane"), ("pid", pid.as_str()), ("pane", &pane.id)],
        ));
    }
    out.family(
        "tilr_process_cpu_seconds_total",
        "counter",
        "CPU time (user and system) of a server process.",
    );
    for (usage, labels) in &processes {
        if let Some(usage) = usage {
            out.sample("tilr_process_cpu_seconds_total", labels, usage.cpu_seconds);
        }
    }
    out.family(
        "tilr_process_memory_bytes",
        "gauge",
        "Memory of a server process: resident (Linux), physical footprint (macOS).",
    );
    for (usage, labels) in &processes {
        if let Some(usage) = usage {
            out.sample("tilr_process_memory_bytes", labels, usage.memory_bytes);
        }
    }
    out.text
}

/// Prometheus text exposition, written by hand.
#[derive(Default)]
struct Exposition {
    text: String,
}

impl Exposition {
    /// `# HELP` and `# TYPE` of a metric family.
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        let help = help.replace('\\', "\\\\").replace('\n', "\\n");
        let _ = writeln!(self.text, "# HELP {name} {help}");
        let _ = writeln!(self.text, "# TYPE {name} {kind}");
    }

    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: impl Display) {
        self.text.push_str(name);
        if !labels.is_empty() {
            self.text.push('{');
            for (i, (label, value)) in labels.iter().enumerate() {
                if i > 0 {
                    self.text.push(',');
                }
                let _ = write!(self.text, "{label}=\"");
                for c in value.chars() {
                    match c {
                        '\\' => self.text.push_str("\\\\"),
                        '"' => self.text.push_str("\\\""),
                        '\n' => self.text.push_str("\\n"),
                        c => self.text.push(c),
                    }
                }
                self.text.push('"');
            }
            self.text.push('}');
        }
        let _ = writeln!(self.text, " {value}");
    }

    fn histogram(&mut self, name: &str, help: &str, histogram: &Histogram) {
        self.family(name, "histogram", help);
        let bucket = format!("{name}_bucket");
        let mut cumulative = 0u64;
        for (i, (_, le)) in BUCKETS.iter().enumerate() {
            cumulative += histogram.counts[i].load(Ordering::Relaxed);
            self.sample(&bucket, &[("le", le)], cumulative);
        }
        cumulative += histogram.counts[BUCKETS.len()].load(Ordering::Relaxed);
        self.sample(&bucket, &[("le", "+Inf")], cumulative);
        let sum = histogram.sum_ns.load(Ordering::Relaxed) as f64 / 1e9;
        self.sample(&format!("{name}_sum"), &[], sum);
        self.sample(&format!("{name}_count"), &[], cumulative);
    }
}

/// CPU time and memory of process `pid`, if it can be read.
#[cfg(target_os = "macos")]
pub fn process_usage(pid: u32) -> Option<Usage> {
    use std::sync::OnceLock;
    // Nanoseconds per unit of `ri_user_time` / `ri_system_time`, which are
    // in mach absolute time (not nanoseconds on Apple silicon).
    static TIMEBASE: OnceLock<f64> = OnceLock::new();
    #[allow(
        deprecated,
        reason = "libc points to the mach2 crate, not a dependency"
    )]
    let timebase = *TIMEBASE.get_or_init(|| {
        let mut info = libc::mach_timebase_info { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid, writable mach_timebase_info.
        let status = unsafe { libc::mach_timebase_info(&mut info) };
        if status == 0 && info.denom != 0 {
            f64::from(info.numer) / f64::from(info.denom)
        } else {
            1.0
        }
    });
    let pid = libc::c_int::try_from(pid).ok()?;
    // SAFETY: an all-zero rusage_info_v4 is a valid value of the plain C
    // struct.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: the buffer is a rusage_info_v4, the struct RUSAGE_INFO_V4 asks
    // the kernel to fill in.
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V4,
            (&raw mut info).cast::<libc::rusage_info_t>(),
        )
    };
    if status != 0 {
        return None;
    }
    let units = info.ri_user_time.saturating_add(info.ri_system_time);
    Some(Usage {
        cpu_seconds: units as f64 * timebase / 1e9,
        memory_bytes: info.ri_phys_footprint,
    })
}

/// CPU time and memory of process `pid`, if it can be read.
#[cfg(target_os = "linux")]
pub fn process_usage(pid: u32) -> Option<Usage> {
    // SAFETY: sysconf has no memory-safety preconditions.
    let (ticks, page) = unsafe {
        (
            libc::sysconf(libc::_SC_CLK_TCK),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    if ticks <= 0 || page <= 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let (utime, stime) = parse_stat_times(&stat)?;
    let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(Usage {
        cpu_seconds: (utime + stime) as f64 / ticks as f64,
        memory_bytes: resident.saturating_mul(page as u64),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_usage(_pid: u32) -> Option<Usage> {
    None
}

/// `utime` and `stime` (fields 14 and 15, in clock ticks) of a
/// `/proc/<pid>/stat` line. The command name (field 2) is in parentheses and
/// may hold spaces and parentheses itself, so the fields are counted from
/// the last `)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_stat_times(stat: &str) -> Option<(u64, u64)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // Field 3 (state) is the first after the name.
    let mut fields = rest.split_whitespace().skip(14 - 3);
    let utime = fields.next()?.parse().ok()?;
    let stime = fields.next()?.parse().ok()?;
    Some((utime, stime))
}

/// Whether the connection on `stream` is a `GET /metrics` request. Looks at
/// the request line without taking it off the stream.
pub async fn is_request(stream: &TcpStream) -> bool {
    // The path metrics are served at.
    const WANT: &[u8] = b"GET /metrics";
    let deadline = tokio::time::Instant::now() + PEEK_TIMEOUT;
    let mut buf = [0u8; WANT.len() + 1];
    loop {
        let n = match stream.peek(&mut buf).await {
            Ok(n) if n > 0 => n,
            _ => return false,
        };
        let seen = n.min(WANT.len());
        if buf[..seen] != WANT[..seen] {
            return false;
        }
        if n > WANT.len() {
            return matches!(buf[WANT.len()], b' ' | b'?');
        }
        // A request line that came in pieces: wait for the rest.
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Read an HTTP request head off `stream`. `None` if it is malformed, too
/// long, or does not come in time.
pub async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    const END: &[u8] = b"\r\n\r\n";
    let mut head = Vec::new();
    let read = async {
        let mut buf = [0u8; 2048];
        loop {
            let n = stream.read(&mut buf).await.ok()?;
            if n == 0 || head.len() + n > MAX_REQUEST {
                return None;
            }
            // Only the new bytes, and the end of the old ones the blank
            // line may have begun in, are searched.
            let from = head.len().saturating_sub(END.len() - 1);
            head.extend_from_slice(&buf[..n]);
            if head[from..].windows(END.len()).any(|w| w == END) {
                return Some(());
            }
        }
    };
    tokio::time::timeout(REQUEST_TIMEOUT, read).await.ok()??;
    parse_request(&head)
}

fn parse_request(head: &[u8]) -> Option<Request> {
    let head = std::str::from_utf8(head).ok()?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let (method, target) = (request_line.next()?, request_line.next()?);
    if !request_line.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let mut builder = Request::builder().method(method).uri(target);
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':')?;
        builder = builder.header(name.trim(), value.trim());
    }
    builder.body(()).ok()
}

/// Answer with `status` and close. `headers` go in besides the length.
pub async fn respond(
    stream: &mut TcpStream,
    status: StatusCode,
    headers: &[(&str, &str)],
    body: &str,
) {
    let mut response = format!(
        "HTTP/1.1 {} {}\r\ncontent-length: {}\r\nconnection: close\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or(""),
        body.len()
    );
    for (name, value) in headers {
        let _ = write!(response, "{name}: {value}\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    // A client that does not read must not hold the task forever.
    let write = async {
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await
    };
    let _ = tokio::time::timeout(RESPONSE_TIMEOUT, write).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            version: "0.1.0-abc \"x\"".into(),
            server_id: "s1".into(),
            clients: 2,
            panes: vec![PaneSnapshot {
                id: "p1".into(),
                pid: 77,
                counters: PaneCounters {
                    frames: 3,
                    frame_bytes: 300,
                    frames_sent: 6,
                    frames_skipped: 2,
                    input: 4,
                    input_bytes: 40,
                },
            }],
        }
    }

    fn rendered(shared: &Shared) -> String {
        render(&snapshot(), shared, 10, |pid| {
            Some(Usage {
                cpu_seconds: f64::from(pid) / 2.0,
                memory_bytes: u64::from(pid) * 1000,
            })
        })
    }

    #[test]
    fn exposition_lists_every_family_with_help_and_type() {
        let shared = Shared::default();
        shared.count_drop(ClientDrop::Superseded);
        let text = rendered(&shared);
        for (name, kind) in [
            ("tilr_server_info", "gauge"),
            ("tilr_server_clients", "gauge"),
            ("tilr_server_panes", "gauge"),
            ("tilr_pane_frames_total", "counter"),
            ("tilr_pane_frame_bytes_total", "counter"),
            ("tilr_pane_frames_sent_total", "counter"),
            ("tilr_pane_frames_skipped_total", "counter"),
            ("tilr_pane_input_total", "counter"),
            ("tilr_pane_input_bytes_total", "counter"),
            ("tilr_client_drops_total", "counter"),
            ("tilr_route_seconds", "histogram"),
            ("tilr_route_input_seconds", "histogram"),
            ("tilr_supervisor_loop_lag_seconds", "histogram"),
            ("tilr_process_cpu_seconds_total", "counter"),
            ("tilr_process_memory_bytes", "gauge"),
        ] {
            assert!(text.contains(&format!("# HELP {name} ")), "{name}:\n{text}");
            assert!(
                text.contains(&format!("# TYPE {name} {kind}\n")),
                "{name}:\n{text}"
            );
        }
        for line in [
            "tilr_server_info{version=\"0.1.0-abc \\\"x\\\"\",server_id=\"s1\"} 1",
            "tilr_server_clients 2",
            "tilr_server_panes 1",
            "tilr_pane_frames_total{pane=\"p1\"} 3",
            "tilr_pane_frame_bytes_total{pane=\"p1\"} 300",
            "tilr_pane_frames_sent_total{pane=\"p1\"} 6",
            "tilr_pane_frames_skipped_total{pane=\"p1\"} 2",
            "tilr_client_drops_total{reason=\"superseded\"} 1",
            "tilr_client_drops_total{reason=\"no_pong\"} 0",
            "tilr_pane_input_total{pane=\"p1\"} 4",
            "tilr_pane_input_bytes_total{pane=\"p1\"} 40",
            "tilr_process_cpu_seconds_total{role=\"supervisor\",pid=\"10\"} 5",
            "tilr_process_cpu_seconds_total{role=\"pane\",pid=\"77\",pane=\"p1\"} 38.5",
            "tilr_process_memory_bytes{role=\"pane\",pid=\"77\",pane=\"p1\"} 77000",
        ] {
            assert!(text.lines().any(|l| l == line), "{line}:\n{text}");
        }
    }

    #[test]
    fn histogram_buckets_are_cumulative() {
        let shared = Shared::default();
        for us in [10, 60, 60, 3_000, 500_000] {
            shared.route.observe(Duration::from_micros(us));
        }
        let text = rendered(&shared);
        for line in [
            "tilr_route_seconds_bucket{le=\"0.00005\"} 1",
            "tilr_route_seconds_bucket{le=\"0.0001\"} 3",
            "tilr_route_seconds_bucket{le=\"0.0025\"} 3",
            "tilr_route_seconds_bucket{le=\"0.005\"} 4",
            "tilr_route_seconds_bucket{le=\"0.1\"} 4",
            "tilr_route_seconds_bucket{le=\"+Inf\"} 5",
            "tilr_route_seconds_sum 0.50313",
            "tilr_route_seconds_count 5",
            "tilr_route_input_seconds_count 0",
        ] {
            assert!(text.lines().any(|l| l == line), "{line}:\n{text}");
        }
    }

    #[test]
    fn stat_times_are_counted_from_the_name() {
        let stat = "123 (a (b) c) S 1 123 123 0 -1 4194560 100 0 0 0 250 75 0 0 20 0 1 0";
        assert_eq!(parse_stat_times(stat), Some((250, 75)));
        assert_eq!(parse_stat_times("123 (x) S 1"), None);
    }

    #[test]
    fn this_process_has_usage() {
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            let usage = process_usage(std::process::id()).expect("usage");
            assert!(usage.memory_bytes > 0);
            assert!(usage.cpu_seconds >= 0.0);
        }
    }

    #[test]
    fn request_head_is_parsed() {
        let request = parse_request(
            b"GET /metrics?token=abc HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer t\r\n\r\n",
        )
        .unwrap();
        assert_eq!(request.uri().path(), "/metrics");
        assert_eq!(request.uri().query(), Some("token=abc"));
        assert_eq!(request.headers()["authorization"], "Bearer t");
        assert!(parse_request(b"GET /metrics\r\n\r\n").is_none());
    }
}
