//! Events: subjects, subscriptions with NATS patterns, delivery to the subscribers
//! connected now (design B6).
//!
//! Owner: `bus-broker-core`, then `bus-streams-events` (events dropped and counted
//! for a lagging subscriber, `tilrd_bus_events_dropped_total`).
//!
//! The broker stores no event: one is delivered to the connections that hold a
//! matching subscription when it is emitted, once per connection, in emit order
//! per emitter, and is gone.

use std::collections::BTreeMap;

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
