//! Method names, event subjects and subscription patterns (design B5, B6).
//!
//! A method is `<namespace>.<name>` and a subject `<segment>(.<segment>)+`: two
//! or more segments of `[a-z0-9_-]+`. A pattern is segments, `*` for exactly one
//! segment, or a trailing `>` for one or more (NATS subjects): `pane.*` matches
//! `pane.bell`, `pane.>` also `pane.x.y`.

/// Namespaces only `tilrd` provides and emits in.
pub const RESERVED_NAMESPACES: &[&str] = &["bus", "pane", "server", "fs", "exec", "clipboard"];

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Whether `name` is a method name or an event subject: two or more segments.
pub fn valid_subject(name: &str) -> bool {
    let mut count = 0;
    for segment in name.split('.') {
        if !valid_segment(segment) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

/// Whether `method` is a method name (the same rule as a subject).
pub fn valid_method(method: &str) -> bool {
    valid_subject(method)
}

/// Whether `pattern` is a subscription pattern.
pub fn valid_pattern(pattern: &str) -> bool {
    let segments: Vec<&str> = pattern.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    let last = segments.len() - 1;
    segments
        .iter()
        .enumerate()
        .all(|(i, segment)| match *segment {
            "*" => true,
            ">" => i == last,
            segment => valid_segment(segment),
        })
}

/// Whether the event `subject` matches the subscription `pattern`.
pub fn matches(pattern: &str, subject: &str) -> bool {
    let mut subject = subject.split('.');
    for part in pattern.split('.') {
        match part {
            ">" => return subject.next().is_some(),
            "*" => {
                if subject.next().is_none() {
                    return false;
                }
            }
            literal => {
                if subject.next() != Some(literal) {
                    return false;
                }
            }
        }
    }
    subject.next().is_none()
}

/// The namespace of a method or subject: its first segment.
pub fn namespace(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

/// Whether `name` lies in a namespace only `tilrd` may provide or emit in.
pub fn is_reserved(name: &str) -> bool {
    RESERVED_NAMESPACES.contains(&namespace(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_and_subjects_have_two_or_more_segments() {
        for good in ["pane.list", "fs.changed.12", "a-b.c_d", "ui.notify"] {
            assert!(valid_subject(good), "{good}");
        }
        for bad in [
            "pane",
            "",
            "pane.",
            ".list",
            "Pane.list",
            "pane.li st",
            "pane..x",
            "pane.*",
        ] {
            assert!(!valid_subject(bad), "{bad}");
        }
    }

    #[test]
    fn patterns_match_as_nats_subjects() {
        assert!(matches("pane.*", "pane.bell"));
        assert!(!matches("pane.*", "pane.x.y"));
        assert!(!matches("pane.*", "pane"));
        assert!(matches("pane.>", "pane.bell"));
        assert!(matches("pane.>", "pane.x.y"));
        assert!(!matches("pane.>", "pane"));
        assert!(matches("test.a", "test.a"));
        assert!(!matches("test.a", "test.a.b"));
        assert!(matches("*.changed.*", "fs.changed.7"));
        assert!(!matches("fs.changed", "pane.changed"));
        assert!(matches("*.>", "anything.at.all"));
    }

    #[test]
    fn a_pattern_is_segments_stars_and_a_trailing_tail() {
        for good in ["pane.*", "pane.>", "*.*", "fs.changed.12", "*.>"] {
            assert!(valid_pattern(good), "{good}");
        }
        for bad in ["pane", ">", "pane.>.x", "pane.**", "pane.", "Pane.*"] {
            assert!(!valid_pattern(bad), "{bad}");
        }
    }

    #[test]
    fn the_core_namespaces_are_reserved() {
        assert!(is_reserved("pane.create"));
        assert!(is_reserved("bus.ping"));
        assert!(!is_reserved("host.open"));
        assert!(!is_reserved("ssh_agent.request"));
        assert!(!is_reserved("panes.x"));
    }
}
