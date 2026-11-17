## ADDED Requirements

### Requirement: A pane is created, listed and closed by calls
`pane.create {cols, rows, command, cwd, profile}` SHALL start a pane as the `panes` and `pane-profiles` specs say and answer its `PaneInfo` once the program runs (cancelled before then, it is closed once it runs); `pane.list {}` SHALL answer the `PaneInfo` of every pane the caller may access; `pane.close {pane_id}` SHALL answer `{exit_code}` once the program ended. An id naming no pane, or one the caller may not access, SHALL get `not_found` `no such pane: <name as sent>`, the connection kept.

**Verified by**: `panes::a_pane_runs_commands_and_reports_how_its_shell_exited`, `panes::asking_for_a_pane_that_does_not_exist_gets_an_error`, `bus_terminal::a_pane_is_created_listed_and_closed_by_calls`, `bus_terminal::failpoints::a_pane_the_caller_may_not_access_is_answered_as_a_missing_one`.

#### Scenario: Create and list
- **WHEN** the owner calls `pane.create {cols: 100, rows: 30}`
- **THEN** the reply's `PaneInfo.pane_id` is a UUID, and `pane.list` lists that pane with `cols` 100 and `rows` 30

#### Scenario: Unknown pane
- **WHEN** the owner calls `pane.close {pane_id: "1"}`
- **THEN** the reply is `not_found` with `no such pane: 1`

#### Scenario: A pane the caller may not access
- **WHEN** the authorizer refuses the caller a running pane and the caller closes, inputs into, resizes, reads or watches it
- **THEN** each reply is `not_found` with `no such pane: <id>`, as for a pane that does not exist, and `pane.list` does not list it

### Requirement: Pane changes are events
The server SHALL emit `pane.created` (`PaneInfo`) when a pane appears, `pane.changed` (`PaneInfo`) when its size, title, directory or viewers change, `pane.title` (`PaneInfo`, which holds `pane_id` and `title`; the frozen schema has no message of its own for it) when its title changes, `pane.bell` and `pane.command_finished {pane_id, at_ms}` for its activity, and `pane.exited {pane_id, exit_code}` when its program ends, each with scope `pane:<id>`.

**Verified by**: `bus_terminal::pane_changes_are_events`, `bus_terminal::a_finished_command_is_a_pane_event`, `panes::a_bell_in_an_unwatched_pane_is_reported`, `panes::a_finished_foreground_command_is_reported`.

#### Scenario: A pane's life
- **WHEN** a subscriber of `pane.*` watches while the owner creates a pane, its shell sets the title to `t`, rings the bell and exits with 3
- **THEN** it receives `pane.created`, `pane.title` with `t`, `pane.bell` and `pane.exited` with `exit_code` 3, in that order

#### Scenario: A finished command
- **WHEN** a subscriber of `pane.command_finished` watches while a command the pane's shell runs in the foreground ends
- **THEN** it receives `pane.command_finished` with the pane's id, an `at_ms` and scope `pane:<id>`

### Requirement: Watching a pane is a stream of frames
`pane.output {pane_id}` SHALL be an interactive stream: a full frame of the pane's screen, then the frames the pane process builds (a `Frame` as `screen-frames` says), and `End{exit_code}` when the program exits, after a full frame of the last screen if the consumer was behind. No chunk SHALL pass the stream's window or `max_frame`: a frame over it goes as its images first; over it without them, the stream ends `too_large`. The caller SHALL be a viewer while the stream is open.

**Verified by**: `reconnect::subscribing_again_sends_the_whole_screen_again`, `panes::two_clients_watch_and_type_into_the_same_pane`, `panes::a_client_that_stops_watching_gets_no_more_frames`, `bus_terminal::watching_a_pane_is_a_stream_of_frames`, `bus_terminal::a_watcher_behind_when_the_pane_exits_gets_its_last_screen`, `bus_terminal::a_pane_call_past_its_deadline_is_answered_timeout_and_cancelled`, `supervisor::bus::core::panes::tests::a_frame_is_split_to_the_streams_window`, `supervisor::bus::core::panes::tests::a_frame_over_max_frame_sends_its_images_first`, `bus_terminal::a_frame_over_the_window_goes_whole_to_a_reader_short_of_credit`.

#### Scenario: Typed command shows
- **WHEN** a client opens `pane.output` on a new pane and inputs `echo "sum is $((6 * 7))"` and Enter
- **THEN** its chunks show `sum is 42`, and `pane.list` lists it in `viewed_by`

#### Scenario: Stop watching
- **WHEN** the client cancels the stream
- **THEN** no more frames arrive and `pane.list` no longer lists it in `viewed_by`

#### Scenario: A frame over the window
- **WHEN** a consumer opens `pane.output` with a 64 KiB window and a frame carries three images of 40 KiB
- **THEN** the images come in chunks of at most 64 KiB, then the frame

#### Scenario: A frame goes whole
- **WHEN** a consumer with a 64 KiB window, crediting each chunk once it read it, watches a pane showing three images of over 32 KiB each
- **THEN** it receives each image once, in a run of image chunks followed by the frame that places them, never images without their frame

#### Scenario: A stream with a deadline
- **WHEN** a consumer opens `pane.output` with `timeout_ms` 300
- **THEN** at the deadline it gets `timeout` and is no viewer any more

### Requirement: A lagging watcher skips to the latest screen
A `pane.output` consumer whose connection's interactive lane has held a packet over 250 ms, or whose stream credit is spent, SHALL have its frames of that pane skipped until it caught up (credit for the frame that did not fit) and SHALL then receive a full frame whose `last_input_seq` is its latest seq a skipped frame echoed; it MUST NOT be dropped for lagging nor hold up other consumers or panes.

**Verified by**: `metrics::a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo`, `reconnect::a_lagging_client_gets_the_last_screen_without_new_output`, itests `UC-049` (`uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others`), `bus_terminal::a_lagging_watcher_skips_to_the_latest_screen`, `bus_terminal::a_slow_reader_of_pane_output_holds_up_no_other_pane_or_reader`, `supervisor::bus::core::panes::tests::a_full_frame_queued_before_a_missed_frame_does_not_end_the_lag`.

#### Scenario: No credit
- **WHEN** a consumer opens `pane.output` with an 8 KiB window, credits nothing while the pane prints 7 MB, then credits again
- **THEN** it receives a full frame matching the pane's screen and stays connected

#### Scenario: A reader that stops reading
- **WHEN** a consumer stops reading its connection while its pane prints 7 MB
- **THEN** the other consumers of that pane and of another pane see their output to the end, and the stopped one, reading again, gets the pane's screen as it is then

#### Scenario: A frame lost after a full one
- **WHEN** a stream's queue holds a full frame and the deltas after it and the next delta does not fit
- **THEN** the full frame goes, the deltas after it do not, and only a newer full frame ends the lag: every delta a consumer gets follows the frame before it

### Requirement: Input is acknowledged after it is applied
`pane.input {pane_id, seq, input_seq, key | mouse | paste | raw | scroll | clear}` and `pane.resize {pane_id, cols, rows, cell_*_px, seq, input_seq}` SHALL be applied in the order a connection sent them and answered once applied (the pty took the last byte, or the scroll, clear or resize took effect), `timeout` past the call's deadline; `seq` SHALL be echoed in `Frame.last_input_seq` (`screen-frames`), with `seq_base` = `HelloAck.connection_id << 32`.

**Verified by**: `input_delivery::disconnect_before_application_replays_one_copy`, `clear_pane::clearing_history_reaches_all_viewers_and_survives_reconnecting`, `bus_terminal::input_is_acknowledged_after_it_is_applied`, `bus_terminal::a_pane_is_resized_by_a_call`, `bus_terminal::keys_mouse_paste_and_scroll_reach_the_pane`, `bus_terminal::a_pane_call_past_its_deadline_is_answered_timeout_and_cancelled`.

#### Scenario: A program that prints nothing
- **WHEN** a client inputs `x` into a pane whose program reads without echoing
- **THEN** the call is answered and no frame is needed for it

#### Scenario: Order kept
- **WHEN** a client sends 100 `pane.input` calls with `raw` `1` to `100` without waiting
- **THEN** the program reads them in that order

#### Scenario: Past the deadline
- **WHEN** a client inputs 1 MiB with `timeout_ms` 300 into a program that does not read
- **THEN** the reply is `timeout` and the connection keeps answering

### Requirement: Numbered input is applied once across reconnects
Input with `input_seq` > 0 SHALL need a `client_id` in `Hello` (else `bad_request: numbered input requires a stable client_id`) and SHALL be applied at most once per `client_id` and pane; `pane.applied {}` SHALL answer within 5 s the caller's applied counters per running pane it sent numbered input to; input naming a missing pane SHALL get a terminal `not_found`, no counter changed. `pane.input_finalize {}` SHALL end the session; an unfinalized one is forgotten an hour after its last connection.

**Verified by**: `input_delivery::applied_input_with_a_lost_ack_is_not_applied_twice`, `input_delivery::numbered_input_requires_a_stable_client_id`, `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect`, `input_delivery::finalized_sessions_do_not_accumulate_delivery_history`, `input_delivery::a_stuck_pane_does_not_block_other_panes_hello`, `bus_terminal::numbered_input_is_applied_once_across_reconnects`, `bus_terminal::failpoints::an_input_whose_reply_was_lost_is_applied_once`, `bus_terminal::a_replayed_input_still_pending_is_written_once`, `bus_terminal::failpoints::a_stuck_pane_is_closed_by_pane_applied`, `bus_terminal::internal_fields_cannot_be_forged`.

#### Scenario: Lost acknowledgement
- **WHEN** a connection ends after input was applied but before its reply arrived, and the client reconnects within an hour and calls `pane.applied`
- **THEN** the answer reports that `input_seq`, and replaying it does not apply it again

#### Scenario: Pending duplicate
- **WHEN** an input is replayed while some of its bytes are still pending for the pty
- **THEN** only the original pending bytes are written and the replayed call is answered after their final byte

#### Scenario: Full outbox
- **WHEN** another input would exceed the client library's 4 MiB or 4096 queued inputs
- **THEN** admission fails visibly, the call is not sent and the queued input stays available for replay

#### Scenario: Finalized and expired sessions
- **WHEN** a client finalizes after all its input was answered, and another disconnects without finalizing for an hour
- **THEN** `pane.applied` reports no counters for either afterwards, numbered input on the finalized connection is refused, and bytes still queued for a pty are written

#### Scenario: A stuck pane does not block the others
- **WHEN** a pane process the client sent numbered input to stops reading its input pipe and the client calls `pane.applied`
- **THEN** within 5 s the stuck pane is closed and the answer carries the counters of the other panes

#### Scenario: Internal fields cannot be forged
- **WHEN** a participant sends `pane.input` with fields the supervisor uses internally towards pane processes
- **THEN** no such field exists in `tilr.pane.v1`, and no other identity's delivery state changes

### Requirement: Reading a pane's text
`pane.read {pane_id, since_seq?, wait_ms?, history_lines?}` SHALL answer the screen as text: rows, `seq`, size, cursor (`x` a cell column), title, cwd, `alive`, `exit_code` and `history_lines` (≤ 10000) of scrollback, fewer past 4 MiB (`too_large`: the screen alone); with `since_seq` it SHALL wait for a change or the exit, ≤ `min(wait_ms, 25 s)` and before the deadline. A pane process silent until the deadline (or 5 s) SHALL give `unavailable`; one ending first, `alive` false and its exit code.

**Verified by**: `bus_terminal::reading_a_pane_and_waiting_for_a_change`, `bus_terminal::a_read_the_pane_process_does_not_answer_ends`.

#### Scenario: Wait for a change
- **WHEN** an instance calls `pane.read {since_seq: S, wait_ms: 5000}` with the current `seq` S and 1 s later a client types into the pane
- **THEN** the answer comes after about 1 s with a `seq` greater than S

#### Scenario: Deadline shorter than the wait
- **WHEN** the call's `timeout_ms` is 500 and `wait_ms` is 5000 and nothing changes
- **THEN** the answer comes before 500 ms with `seq` S, not a `timeout`
