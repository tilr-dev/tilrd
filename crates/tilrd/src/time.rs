//! Measurement clocks. A `failpoints` test may publish one shared timestamp
//! file for the supervisor and its separately spawned pane processes.

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// Monotonic reading used only for pane timing measurements. Scheduling and
/// operational deadlines continue to use the real `Instant` clock.
pub fn measurement_instant() -> Instant {
    #[cfg(feature = "failpoints")]
    if let Some(us) = test_time_us() {
        use std::sync::OnceLock;
        use std::sync::atomic::{AtomicU64, Ordering};
        static BASE: OnceLock<Instant> = OnceLock::new();
        static LAST_US: AtomicU64 = AtomicU64::new(0);
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

/// Unix milliseconds used for `PaneInfo` creation and last use.
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
