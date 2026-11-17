//! Events: subjects, subscriptions with NATS patterns, delivery to the subscribers
//! connected now (design B6).
//!
//! Owner: `bus-broker-core`, then `bus-streams-events` (events dropped and counted
//! for a lagging subscriber, `tilrd_bus_events_dropped_total`).
//!
//! The broker stores no event: one is delivered to the connections that hold a
//! matching subscription when it is emitted, once per connection, in emit order
//! per emitter, and is gone. A subscriber whose interactive lane lags (a packet
//! held over 250 ms, or the lane as full as a call may fill it) does not get it:
//! the event is dropped and counted by subject ([`count_dropped`], the metric
//! `tilrd_bus_events_dropped_total`), and the connection stays.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Subjects counted apart in `tilrd_bus_events_dropped_total`; the others are
/// counted under [`OTHER_SUBJECTS`], so a flood of subjects does not grow it.
const COUNTED_SUBJECTS: usize = 1000;
/// The `subject` label of the drops beyond [`COUNTED_SUBJECTS`].
pub const OTHER_SUBJECTS: &str = "other";

/// Events dropped for lagging subscribers, by subject, since the start.
static DROPPED: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

/// `count` deliveries of an event of `subject` were dropped.
pub fn count_dropped(subject: &str, count: u64) {
    let mut dropped = DROPPED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = if dropped.contains_key(subject) || dropped.len() < COUNTED_SUBJECTS {
        subject
    } else {
        OTHER_SUBJECTS
    };
    *dropped.entry(key.to_owned()).or_default() += count;
}

/// The drops so far, by subject, sorted.
pub fn dropped() -> Vec<(String, u64)> {
    DROPPED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|(subject, count)| (subject.clone(), *count))
        .collect()
}

/// The subscriptions of one connection, by the id it chose.
#[derive(Debug, Default)]
pub struct Subscriptions {
    by_id: BTreeMap<u64, String>,
}

impl Subscriptions {
    /// Add a subscription; `false` if the id is taken.
    pub fn add(&mut self, id: u64, pattern: &str) -> bool {
        if self.by_id.contains_key(&id) {
            return false;
        }
        self.by_id.insert(id, pattern.to_owned());
        true
    }

    pub fn remove(&mut self, id: u64) -> Option<String> {
        self.by_id.remove(&id)
    }

    /// Whether one of the patterns matches `subject`: the event is delivered once.
    pub fn matches(&self, subject: &str) -> bool {
        self.by_id.values().any(|pattern| matches(pattern, subject))
    }

    pub fn patterns(&self) -> impl Iterator<Item = &str> {
        self.by_id.values().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Whether the event `subject` matches the subscription `pattern`: segments, `*`
/// for exactly one, a trailing `>` for one or more.
pub fn matches(pattern: &str, subject: &str) -> bool {
    tilr_bus::names::matches(pattern, subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_as_nats_subjects() {
        assert!(matches("test.*", "test.a"));
        assert!(!matches("test.*", "test.a.b"));
        assert!(matches("test.>", "test.a"));
        assert!(matches("test.>", "test.a.b"));
        assert!(!matches("test.>", "test"));
        assert!(matches("pane.*", "pane.bell"));
        assert!(matches("*.changed.*", "fs.changed.7"));
        assert!(!matches("test.a", "test.b"));

        let mut subscriptions = Subscriptions::default();
        assert!(subscriptions.add(1, "test.*"));
        assert!(subscriptions.add(2, "test.>"));
        assert!(!subscriptions.add(2, "other.*"), "an id is taken once");
        // Two matching patterns: still one delivery, which `matches` decides.
        assert!(subscriptions.matches("test.a"));
        assert!(subscriptions.matches("test.a.b"));
        assert!(!subscriptions.matches("other.a"));
        assert_eq!(subscriptions.remove(2).as_deref(), Some("test.>"));
        assert!(!subscriptions.matches("test.a.b"));
    }
}
