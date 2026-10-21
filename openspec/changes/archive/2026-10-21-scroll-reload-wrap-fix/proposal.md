# Fix the scroll reload test's frame synchronization

## Why

The reload scroll test intermittently misses `line-2` after resizing and scrolling a
reloaded pane. The test waits for reflowed text, which may already be visible in
the resize frame before the scroll has been applied.

## What Changes

- Deliver and observe the resize frame before requesting the scroll, to exercise
  that ordering deterministically.
- Wait for the requested viewport before asserting its history contents.

## Capabilities

No requirement changes. This strengthens verification of reload's existing
"Soft wraps carry over" / "Wrapped line at the border" scenario.

## Impact

Only `crates/tilrd/tests/scroll.rs`; no production or protocol changes.
