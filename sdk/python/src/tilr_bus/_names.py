"""Method names, event subjects and subscription patterns (design B5, B6;
`crates/tilr-bus/src/names.rs`)."""

from __future__ import annotations

import re

_SEGMENT = re.compile(r"[a-z0-9_-]+")


def valid_subject(name: str) -> bool:
    """Whether `name` is a method name or an event subject: two or more segments."""
    segments = name.split(".")
    return len(segments) >= 2 and all(_SEGMENT.fullmatch(s) for s in segments)


valid_method = valid_subject


def valid_pattern(pattern: str) -> bool:
    """Whether `pattern` is a subscription pattern: segments, `*`, a trailing `>`."""
    segments = pattern.split(".")
    if len(segments) < 2:
        return False
    last = len(segments) - 1
    return all(
        s == "*" or (s == ">" and i == last) or _SEGMENT.fullmatch(s)
        for i, s in enumerate(segments)
    )


def matches(pattern: str, subject: str) -> bool:
    """Whether the event `subject` matches the subscription `pattern`."""
    parts = subject.split(".")
    i = 0
    for part in pattern.split("."):
        if part == ">":
            return i < len(parts)
        if i >= len(parts) or (part != "*" and part != parts[i]):
            return False
        i += 1
    return i == len(parts)
