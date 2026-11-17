# Design: The core's pane methods on the bus

Shared design: `openspec/design/unified-bus.md` §T1 (the core as a native participant), §T2
(mapping), §T3 (guarantees), §T4 (`pane.output`), §T5 (input), §T9 (`pane.read`), §B9 (JSON,
compressed chunks). This change's part follows.

## Decisions

- **Through the handle.** Every method runs as `CoreCmd::With` closures on the supervisor's
  existing paths (create, input with acknowledgement, resize, close, ask for a full frame); no
  pane logic is duplicated. Hooks in `supervisor.rs` *(added by the split)*: `on_pane_output`
  also offers each frame to the `BusObserver`s; `ask_full_frame` takes a bus consumer as well
  as a `tilr.v1` subscriber; the pane list push also calls `pane_event`. A `pane.output`
  consumer is a viewer (`PaneInfo.viewed_by`) for the stream's life.
- **Tests on the bus now, the old names later.** The requirements of this change name, besides
  the new `bus_terminal::*` tests, tests of today's files (`panes::*`, `reconnect::*`,
  `input_delivery::*`, `metrics::*`, UC-049) that still speak `tilr.v1` until
  `bus-test-clients` moves them; until then the `bus_terminal::*` tests are the bus's check and
  the old ones keep checking `tilr.v1`. `bus_terminal::a_lagging_watcher_skips_to_the_latest_screen`
  *(added by the split)* covers the lag rule on the bus meanwhile.
- **Listing.** The `tilrd`/`pane.output` clause of `bus::participants_and_methods_are_listed`
  (`bus-instances`) is added here.

## Reload and reconnect

Panes, screens and pending input cross a reload as today; bus participants reconnect with a new
`connection_id`, reopen `pane.output` (full frame first), call `pane.applied` and replay
unacknowledged input (T5). `created_by` is carried by `bus-reload`.

## Backpressure

`pane.output` is interactive and latest-wins (T4); input calls are bounded by `max_in_flight`
and the client outbox (T5); everything else follows B7/B8 of `bus-streams-events`.
