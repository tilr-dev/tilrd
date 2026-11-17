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
- **Broker files untouched** *(coordinator's decision after the review of the broker)*: this
  change does not edit `bus/broker.rs` or `bus/transport.rs` (after `bus-broker-core` only
  `bus-streams-events`, then `bus-reload`, do); it uses the broker's API: the `BrokerHandle`, `StreamOut`, and
  `BusObserver` (called with `&Supervisor`).

## Implementation notes

Where the implementation differs from or adds to the decisions above:

- **Supervisor hooks.** The foundation's observer does not see input acks, a pane process that
  ends before its program runs, a pane list that changed, or a text reply; four calls in
  `supervisor.rs` (each only while the bus observes) go to `bus/core/panes.rs`:
  `input_ack` (end of `on_input_ack`), `create_failed` (the starting branch of
  `on_pane_link_closed`), `panes_changed` (`broadcast_pane_list`) and `text_reply` (the
  pane link's new `kind::TEXT`).
- **Text from the pane process.** `pane.read` asks the pane process for its text: the
  supervisor sends `link::READ_TEXT` (request id, history lines), the pane answers
  `kind::TEXT` with a `pane.ReadReply`, rendered by `tilr_screen::Screen::text`.
- **Input acknowledged when applied.** Every bus input goes as reliable delivery input
  (identity and seq) so the pane process acknowledges it once applied: numbered input under
  an identity derived from (bus, identity, client id), the rest under one identity per
  connection with the core's counter. `pane.applied` probes, with an empty delivery input
  above every `tilr.v1` client id, only the panes the caller's numbered identity sent input
  to (unlike `tilr.v1`'s hello, which asks every pane): a pane it never sent any to has no
  counter of it and is not in the answer. A probed pane that does not answer in 5 s is
  killed, as `tilr.v1`'s hello does. Which panes an identity sent to is kept by the core
  and does not cross a reload (as `created_by`); after one, `pane.applied` answers no
  counters and a replay is still applied once, the pane process keeping its counters. Once
  rights tell input from close (`bus-pane-rights`), the kill needs the right to close the
  pane, not only to type into it.
- **Pending acknowledgements.** `pane.input_finalize` answers the calls of its identity
  still waiting for an acknowledgement with `unavailable` (the pane processes forget it);
  the sweep does so for the calls of a connection that ended, and drops waits whose caller
  cancelled.
- **`pane.title`.** The schema has no message for it (and is frozen); its payload is the
  pane's `PaneInfo`, which holds `pane_id` and `title`.
- **Existence.** A pane the caller may not access is answered `not_found` (`no such pane`)
  like a pane that does not exist, so a refusal does not tell that it exists; without a
  broker the core refuses every call `unavailable`. This build's authorizer refuses no
  `PaneAccess` a call can reach (a call with a credential is refused `denied` by the broker
  before its method runs), so the test stands in for one that does with the failpoint
  `bus-pane-access` (`failpoints` builds only).
- **The last screen.** A pane process's last frame, sent when its program ended, is full,
  for `tilr.v1` too: a `pane.output` consumer behind then (lagging, waiting for its full
  frame) gets the last screen whole before `End{PaneExited}`; a full frame that did not fit
  the stream's queue is kept for that. A reader that never credits holds that last frame
  until its stream is cancelled, its deadline passes or its connection ends: one frame per
  such stream.
- **A lost full frame.** A frame that did not fit a stream's queue makes it skip and ask for
  a full frame again once the consumer caught up, also when the frame lost was the full one
  it waited for. The stream keeps the number of the latest frame that did not fit: a full
  frame queued before it is sent but does not end the lag (the deltas after the lost frame
  assume a newer screen), so a newer full frame is asked for.
- **`pane.copied`** is emitted by `bus-core-services`, which owns the clipboard; this change
  emits the other pane events.
- **`pane.read` and the pane process.** The text is awaited for what the call has left, or
  5 s without a deadline; a pane process that does not answer (stuck, an old binary after a
  failed self-reload, a request lost in its exec) gets `unavailable`. A pane that ends
  before it answered gives `alive` false with its exit code. The text is at most
  `tilr_screen::MAX_TEXT_BYTES` (4 MiB): rows of history past it are left out, the oldest
  first; a viewport alone over it is `too_large`. Without the screen's scrollbar only the
  viewport is read. `Cursor.x` is a cell column.
- **A cancelled `pane.create`.** The pane is started all the same; when its caller is gone
  by the time its program runs, it is closed at once (after its `pane.created`).
- **`pane.create` on its own.** It is a method of its own, given the call, and
  `panes::start_pane` takes the environment to add (`Supervisor::create_pane` adds it to
  every pane's), so `pane-profiles` can resolve a profile before it and pass what it adds.
- **Viewers.** `Viewer.address` is the transport (`unix`, `tcp`, `instance`): the broker's
  principal has no peer address.
- **`pane.read` deadline.** The wait is bounded by the call's deadline, which the broker
  gives every unary call (`call_timeout_ms`, 10 s by default): the answer comes 100 ms
  before it, the text as it is then, not `timeout`.
- **Sessions of numbered input** are forgotten when the broker tells their connection ended
  (`BrokerHandle::watch_connection`, at once when it is not connected any more); an
  acknowledgement wait whose call was cancelled or timed out goes with the next input of
  its identity to that pane, or with the acknowledgement.
- **Cancelled handlers.** The broker aborts a native handler when its caller cancels; the
  watcher of `pane.output` and the waiter of `pane.read` are dropped by guards, not by code
  after an `await`.
- **Listing.** The `tilrd` clause of `bus::participants_and_methods_are_listed` waits for
  `bus-instances`: that test is not in this change's base.
- **A broker test changed.** `bus::unknown_envelopes_are_ignored_and_bad_ones_reported` called
  `pane.list` to get `not_found` from a broker without pane methods; it calls `bus.absent` now,
  which the participant `tilrd` does not provide either.
- **More tests.** `bus_terminal::numbered_input_is_applied_once_across_reconnects` covers
  numbered input, `pane.applied` and `pane.input_finalize` on the bus;
  `bus_terminal::a_pane_is_resized_by_a_call`,
  `bus_terminal::keys_mouse_paste_and_scroll_reach_the_pane`,
  `bus_terminal::a_replayed_input_still_pending_is_written_once`,
  `bus_terminal::internal_fields_cannot_be_forged`,
  `bus_terminal::a_read_the_pane_process_does_not_answer_ends`,
  `bus_terminal::a_watcher_behind_when_the_pane_exits_gets_its_last_screen`,
  `bus_terminal::failpoints::an_input_whose_reply_was_lost_is_applied_once` and
  `bus_terminal::failpoints::a_stuck_pane_is_closed_by_pane_applied` the rest of the
  scenarios on the bus.
- **Chunks to the window.** No `pane.output` chunk is larger than the stream's window
  (`StreamOut::window`, at most `max_frame` less 1 KiB for the envelope): a frame over it
  goes as its images first, in chunks of at most the window, then the frame; an image over
  it is left out, as one over `max_frame` is; a frame over it without its images cannot go
  and ends the stream `too_large` (the broker would end it so on the send).
- **A frame goes whole.** When the first chunk of a frame finds no credit (`try_send` is
  `Full`), the frame is skipped (latest-wins); once one chunk of it went, the rest waits for
  credit (`send`), so a consumer never sees images without their frame. Only that stream's
  task waits: the observer hands each stream at most 16 frames and never waits, so a slow
  reader holds up no other reader and no pane.
- **Credit before a full frame.** A consumer behind is asked a full frame once it caught up
  for the chunk that did not fit (`StreamOut::caught_up_for(bytes)`), not for one byte: a
  full frame larger than the credit back is not asked for again and again.
- **Unary replies are not chunked.** The replies of `pane.read` (at most
  `tilr_screen::MAX_TEXT_BYTES`, 4 MiB, `too_large` past `max_frame`) and of the other
  methods are one `Reply` each; the interactive lane's bound (32 MiB, then 4001) is the
  caller's to keep with `max_in_flight`.
- **No `know_methods`.** `pane.*` are methods of the native participant `tilrd`, installed
  before the broker serves; they never lack a provider while the bus runs, so the broker's
  list of methods known without a provider (`BrokerHandle::know_methods`, `unavailable`
  instead of `not_found`) does not name them, and the spec asks for no such answer.
- **Deadlines.** A pane call past its deadline is answered `timeout` by the broker, which
  aborts the handler: an input's wait is dropped (above), a `pane.output` watcher goes with
  its guard (the caller is no viewer any more).
- **Profiles: the seam.** `pane.create` does not resolve `profile`; `pane-profiles` adds it
  in `create_method` (`bus/core/panes.rs`) before the supervisor's part: it resolves the
  profile with the call's `CallCtx`, then queues `create(s, &caller, request, env)`, which
  passes `env`, the environment the profile adds, to `start_pane`
  (`Supervisor::create_pane` adds it to every pane's). Today `env` is empty.

## Reload and reconnect

Panes, screens and pending input cross a reload as today; bus participants reconnect with a new
`connection_id`, reopen `pane.output` (full frame first), call `pane.applied` and replay
unacknowledged input (T5). `created_by` is carried by `bus-reload`.

## Backpressure

`pane.output` is interactive and latest-wins (T4); input calls are bounded by `max_in_flight`
and the client outbox (T5); everything else follows B7/B8 of `bus-streams-events`.
