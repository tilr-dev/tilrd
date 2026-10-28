//! Measurement clocks. A `failpoints` test may publish one shared timestamp
//! file for the supervisor and its separately spawned pane processes.

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

#[cfg(feature = "failpoints")]
static LAST_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(feature = "failpoints")]
#[derive(Clone, Copy)]
pub enum FrameStage {
    Parse,
    Build,
    Encode,
}

/// Advance only the measurement clock, after the named operation and before
/// its ending reading. The ending reading supplies the remaining one tick.
#[cfg(feature = "failpoints")]
pub fn advance_frame_stage(stage: FrameStage) {
    let Some(path) = std::env::var_os("TILR_TEST_FRAME_STAGE_TIME_FILE") else {
        return;
    };
    assert!(
        test_time_file().is_some(),
        "frame stage timing requires TILR_TEST_TIME_FILE"
    );
    let text = std::fs::read_to_string(path).expect("read TILR_TEST_FRAME_STAGE_TIME_FILE");
    let durations: Vec<u64> = text
        .split_whitespace()
        .map(|value| {
            value
                .parse()
                .expect("frame stage durations must be unsigned decimal microseconds")
        })
        .collect();
    assert!(
        durations.len() == 3 && durations.iter().all(|us| *us > 0),
        "frame stage timing requires three positive durations: parse build encode"
    );
    LAST_US
        .try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |us| us.checked_add(durations[stage as usize] - 1),
        )
        .expect("frame stage measurement clock overflow");
}

/// Monotonic reading used only for pane timing measurements. Scheduling and
/// operational deadlines continue to use the real `Instant` clock.
pub fn measurement_instant() -> Instant {
    #[cfg(feature = "failpoints")]
    if let Some(us) = test_time_us() {
        use std::sync::OnceLock;
        use std::sync::atomic::Ordering;
        static BASE: OnceLock<Instant> = OnceLock::new();
        // A frame build and its encode must take at least one clock tick even
        // while the shared file is unchanged. The file still sets the next
        // larger value, and readings never go backwards.
        let mut previous = LAST_US.load(Ordering::Relaxed);
        let logical_us = loop {
            let next = us.max(previous.saturating_add(1));
            match LAST_US.compare_exchange_weak(
                previous,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break next,
                Err(actual) => previous = actual,
            }
        };
        return BASE
            .get_or_init(Instant::now)
            .checked_add(std::time::Duration::from_micros(logical_us))
            .expect("TILR_TEST_TIME_FILE timestamp exceeds Instant range");
    }
    Instant::now()
}

/// Monotonic time for activity rate limits, with an exact test clock.
pub fn activity_instant() -> Instant {
    #[cfg(feature = "failpoints")]
    if let Some(us) = test_time_us() {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        return BASE
            .get_or_init(Instant::now)
            .checked_add(std::time::Duration::from_micros(us))
            .expect("TILR_TEST_TIME_FILE timestamp exceeds Instant range");
    }
    Instant::now()
}

/// Unix milliseconds used for pane metadata and activity timestamps.
pub fn now_ms() -> u64 {
    #[cfg(feature = "failpoints")]
    if let Some(us) = test_time_us() {
        return us / 1_000;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[cfg(feature = "failpoints")]
pub fn test_time_file() -> Option<std::path::PathBuf> {
    std::env::var_os("TILR_TEST_TIME_FILE").map(Into::into)
}

#[cfg(feature = "failpoints")]
pub fn test_time_us() -> Option<u64> {
    let path = test_time_file()?;
    let value = std::fs::read_to_string(&path).expect("read TILR_TEST_TIME_FILE");
    Some(
        value
            .trim()
            .parse()
            .expect("TILR_TEST_TIME_FILE must contain unsigned decimal microseconds"),
    )
}

/// One explicitly signaled test-clock deadline. The caller keeps its real
/// upper bound and acknowledges only after selecting this expiration path.
#[cfg(feature = "failpoints")]
pub struct TestDeadline {
    control: tokio::net::TcpListener,
    deadline_us: u64,
}

#[cfg(feature = "failpoints")]
impl TestDeadline {
    pub async fn new(name: &str, duration: std::time::Duration) -> Option<Self> {
        let path = test_time_file()?;
        let start_us = test_time_us().expect("test clock path is set");
        let control = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test deadline control");
        let deadline = Self {
            deadline_us: start_us.saturating_add(duration.as_micros() as u64),
            control,
        };
        std::fs::write(
            path.with_extension(name),
            deadline
                .control
                .local_addr()
                .expect("control address")
                .port()
                .to_string(),
        )
        .expect("publish test deadline control");
        // This event proves both the start timestamp and listener are ready.
        tracing::info!("test deadline ready: {name}");
        Some(deadline)
    }

    pub async fn expired(&self) -> tokio::net::TcpStream {
        loop {
            let (signal, _) = self.control.accept().await.expect("test deadline signal");
            if test_time_us().expect("test clock path is set") >= self.deadline_us {
                return signal;
            }
        }
    }

    pub async fn acknowledge(mut signal: tokio::net::TcpStream) {
        use tokio::io::AsyncWriteExt;
        signal
            .write_all(&[1])
            .await
            .expect("acknowledge test deadline");
    }
}
