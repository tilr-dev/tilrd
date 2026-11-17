# Design: Move the test client, the harness, the tests and itests to the bus

Shared design: `openspec/design/unified-bus.md` §T10, §T2 (what each message becomes), O2, O4.
This change's part follows.

## Decisions

- **Same helper names.** `common::Client` keeps its methods (`create_pane`, `subscribe`,
  `type_keys`, ...), implemented with `tilr-bus` calls and streams, so test bodies change little
  and names stay.
- **Raw `tilr.v1` only where the protocol is the subject.** The four protocol tests named in the
  proposal keep a minimal raw path in `tests/common/v1.rs` until `remove-tilr-v1`.
- **One owner of `tests/common/mod.rs`.** No other change edits it before this one merges (O4);
  the lanes' `bus_*.rs` helpers are kept as they are.

## Reload and reconnect

The tests of reload and reconnect keep their behavior; on the bus they reopen streams and call
`pane.applied` (T5).

## Backpressure

Unchanged: the tests that check lanes and lag (`metrics::*`, `files::*`) now measure the bus's
credits and lanes (B7).
