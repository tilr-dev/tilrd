# Publish applied resize dimensions before input acknowledgement

## Why
A numbered Resize takes effect immediately, but its cumulative delivery acknowledgement waits for earlier PTY input. The supervisor currently learns the new dimensions only from that acknowledgement, leaving PaneList stale while the PTY is blocked.

## What Changes
- Publish the applied dimensions independently of cumulative input delivery progress, using the existing pane acknowledgement channel and the previously applied input sequence.
- Keep acknowledgement of the numbered Resize behind earlier PTY writes.
- Add a deterministic regression with a program blocked on an external FIFO.

## Impact
The pane process publishes current dimensions for every applied Resize. No protocol or reload format changes.
