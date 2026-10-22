# Preserve pending output in input acknowledgement frames

## Why
The metrics test observes `two\n` before the shell necessarily prints its next prompt. A no-op scroll can then be credited while PTY output is pending, and its frame correctly includes the accumulated screen diff. Expecting an empty diff at that point races with the shell.

## What Changes
- Limit the empty-frame guarantee to input processed without accumulated screen changes or pending output.
- Keep output coalescing and the latest credited input sequence in the resulting frame.
- Wait for the prompt after `two` before asserting an empty scroll frame, and control the output/scroll order in a regression test.

## Impact
Metrics requirements and tests. No wire format or production coalescing change.
