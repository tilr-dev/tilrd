# The terminal on the bus

## Purpose
The participant `tilrd` serves panes on `tilr.bus.v1` as on `tilr.v1`: `pane.*` calls create,
list, close, resize and type into panes, `pane.output` streams a pane's frames as the pane
process encoded them, paced by the stream's window and credit, `pane.read` reads a pane as
text, and `pane.*` events scoped to the pane tell what changed.

## Requirements

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

**Verified by**: `bus_terminal::reading_a_pane_and_waiting_for_a_change`, `bus_terminal::a_read_the_pane_process_does_not_answer_ends`, `supervisor::bus::core::panes::tests::the_text_after_a_change_is_waited_for_within_the_reads_wait`.

#### Scenario: Wait for a change
- **WHEN** an instance calls `pane.read {since_seq: S, wait_ms: 5000}` with the current `seq` S and 1 s later a client types into the pane
- **THEN** the answer comes after about 1 s with a `seq` greater than S

#### Scenario: Deadline shorter than the wait
- **WHEN** the call's `timeout_ms` is 500 and `wait_ms` is 5000 and nothing changes
- **THEN** the answer comes before 500 ms with `seq` S, not a `timeout`

### Requirement: tilrd provides the terminal's methods on the bus
The participant `tilrd` SHALL be the provider of the pane methods and of `server.info`, `server.reload`, `server.update`, `server.command`, `fs.list`, `fs.stat`, `fs.read`, `fs.write`, `fs.mkdir`, `fs.rename`, `fs.delete`, `fs.watch`, `fs.unwatch`, `exec.run`, `exec.input`, `exec.signal`, `clipboard.put` and `clipboard.get` on a server started with default options; `fs.read`, `exec.run`, `clipboard.get`, `server.update` and `server.command` SHALL be streams on the bulk lane.

**Verified by**: `bus_terminal::services::tilrd_provides_the_terminal_methods`.

#### Scenario: Methods listed
- **WHEN** the owner calls `bus.methods` on a server started with default options
- **THEN** `tilrd` is the only provider of `pane.create`, `pane.list`, `pane.close`, `pane.resize`, `pane.input`, `pane.read`, `pane.applied`, `pane.input_finalize`, `pane.output` and each service method above, and `fs.read`, `exec.run`, `clipboard.get`, `server.update` and `server.command` are listed as bulk streams

### Requirement: server.info describes the server
`server.info {}` SHALL answer `server_version`, `update_channel` (from `server.toml`, default `stable`), `server_id` (a UUID kept in `server-id`), `features`, `clipboard_max_bytes` and `fs_max_bytes` (0 without their feature), as `tilr.v1`'s `HelloAck` does. `features` SHALL list what this server offers of `clipboard`, `files`, `host-events`, `agent` and `exec`, in this order; the methods of a feature not listed MUST NOT be provided.

**Verified by**: `bus_terminal::services::server_info_describes_the_server`, `panes::hello_reports_the_configured_update_channel`, `panes::a_server_keeps_its_id_when_it_restarts`, `exec::exec_is_offered_unless_turned_off`.

#### Scenario: Everything on
- **WHEN** a server started with default options is asked `server.info`
- **THEN** `features` is `["clipboard", "files", "host-events", "agent", "exec"]`, and `server_id`, `server_version`, `clipboard_max_bytes` and `fs_max_bytes` are those of its `HelloAck`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec --no-files`
- **THEN** `exec` and `files` are missing from `features`, `fs_max_bytes` is 0, and `exec.run` and `fs.list` are answered `not_found`

### Requirement: Files on the bus
`fs.list`, `fs.stat`, `fs.mkdir`, `fs.rename` and `fs.delete` SHALL be calls answering `FsEntries`, `FsInfo` and `FsAck`; paths, roots, read-only mode and the `*_bytes` twins SHALL behave as the `files` spec says, and a refusal SHALL be `bad_request` with the message `tilr.v1` gives.

**Verified by**: `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `bus_terminal::services::a_path_out_of_the_roots_is_refused_on_the_bus`, `bus_terminal::services::a_read_only_server_refuses_changes_on_the_bus`.

#### Scenario: Outside the roots
- **WHEN** a client calls `fs.list {path: "/"}` on a server that shares its home directory
- **THEN** the reply is `bad_request` with a message saying the path is outside the directories the server shares

#### Scenario: Out through a link or `..`
- **WHEN** a client lists `~/escape`, a link in the home directory to the directory above it, or `~/..`, or renames a file of the home directory to the directory above it or through that link
- **THEN** each reply is `bad_request` saying the path is outside the directories the server shares, and the file stays

#### Scenario: Read-only
- **WHEN** the server runs with `[files] readonly = true` and a client calls `fs.write`, `fs.mkdir`, `fs.rename` or `fs.delete`
- **THEN** each reply is `bad_request` saying files are read-only, nothing changes, and `fs.list` still answers

### Requirement: Watching files on the bus
`fs.watch {path}` SHALL answer `{watch_id}` and emit `fs.changed.<watch_id>` events until `fs.unwatch {watch_id}` or the end of the caller's connection; `fs.unwatch` of a watch that is not the caller's, or of none, SHALL be `not_found`.

**Verified by**: `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `bus_terminal::services::a_watch_is_unwatched_only_by_its_connection`, `files::a_watched_directory_reports_what_changes_in_it`.

#### Scenario: Watch a directory
- **WHEN** a client calls `fs.watch` on a directory, subscribes to `fs.changed.*`, and a file is written there
- **THEN** it receives an `fs.changed.<watch_id>` event naming the file with kind create

#### Scenario: Unwatched twice
- **WHEN** a client calls `fs.unwatch` with its watch's id twice
- **THEN** the first reply is `FsAck` and the second `not_found`

#### Scenario: Another connection's watch
- **WHEN** a connection calls `fs.unwatch` with the id of a watch another connection made
- **THEN** the reply is `not_found`, and the watch's own connection still unwatches it

### Requirement: File content moves in chunks
`fs.read` SHALL be a stream on the bulk lane of `FsData` chunks of at most 256 KiB and at most the stream's window, ending with an error when the read fails; `fs.write` SHALL take the content in calls of one chunk each sharing a `transfer_id`, replace the file only after the `last` chunk, and answer each chunk of a failed transfer with its error: a `total_size` over `fs_max_bytes`, or more data than `total_size` across the chunks, fails it.

**Verified by**: `bus_terminal::services::file_content_moves_in_chunks`, `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `bus_terminal::services::an_upload_is_held_to_its_size_and_the_servers_limit`, `files::a_file_is_written_whole_or_not_at_all`.

#### Scenario: Read a large file in a small window
- **WHEN** a client reads a 4 MiB file with `fs.read` and a 256 KiB window
- **THEN** no chunk is larger than the window and the whole file arrives intact

#### Scenario: Written after the last chunk
- **WHEN** a client sends the first of two `fs.write` chunks of a new file
- **THEN** the file is not there until the second, `last`, chunk is answered

#### Scenario: More than announced
- **WHEN** a client writes a new file with `total_size` 3 in the chunks `ab` and `cd` (`last`)
- **THEN** the second reply is `bad_request` saying the write carries more data than it announced, the file is not there and no temporary file is left

#### Scenario: Over the server's limit
- **WHEN** a first `fs.write` chunk has a `total_size` of `fs_max_bytes` plus one
- **THEN** the reply is `bad_request` saying it is too large

### Requirement: Exec on the bus
`exec.run {argv, cwd, env, stdin, timeout_ms}` SHALL be a bulk stream: `started {exec_id, pid}` first (`spawn_failed` if it did not start), `output {stream, data}` chunks in order per stream, `End` with the `ExecExit` (`code`, `signal`, `timed_out`, `cancelled`, `reason`). Cancelling the stream at any point, or ending the connection, SHALL kill the program's group; a reader that credits slowly SHALL hold the program back.

**Verified by**: `bus_terminal::services::a_program_runs_on_the_bus_and_its_outputs_come_apart`, `bus_terminal::services::a_cancelled_program_ends_with_its_group`, `bus_terminal::services::failpoints::a_program_cancelled_as_it_starts_ends_with_its_group`, `bus_terminal::services::a_reader_that_credits_slowly_holds_the_program_back`, `bus_terminal::services::what_a_connection_made_goes_when_it_ends_or_is_superseded`.

#### Scenario: Outputs apart
- **WHEN** a client runs `exec.run {argv: ["/bin/sh", "-c", "echo out; echo err >&2; exit 3"]}`
- **THEN** it receives `started`, `out` on stream 1, `err` on stream 2, and `End` with code 3

#### Scenario: Cancel
- **WHEN** a client runs a program whose child holds a FIFO open and cancels the stream
- **THEN** the program's process group is killed and the FIFO's writer is gone

#### Scenario: Cancelled as it starts
- **WHEN** the stream is cancelled after the program started and before the stream's handler took up its start
- **THEN** the program's process group is killed all the same

#### Scenario: A slow reader
- **WHEN** a client runs a program that writes 16 MiB to stdout and then makes a file, with a 64 KiB window, and reads the stream one chunk at a time
- **THEN** the file is not there before the client has read all but at most 1 MiB of the output

### Requirement: Exec input, signals and server commands
`exec.input {exec_id, data, eof}` SHALL be answered with `{exec_id, taken}` once the program took the bytes, and more than 4 MiB sent and not yet answered SHALL kill it (`reason` `input overrun`, the call `bad_request`); `exec.signal {exec_id, signal}` SHALL signal its group; for both, a program of another connection, or none, SHALL be `not_found`. `server.command {argv}` SHALL run the server's own executable as `exec.run` does, for the owner on the Unix socket only, and be `denied` otherwise.

**Verified by**: `bus_terminal::services::a_program_runs_on_the_bus_and_its_outputs_come_apart`, `bus_terminal::services::exec_input_is_the_callers_and_overrun_kills_the_program`, `bus_terminal::services::server_commands_are_taken_on_the_unix_socket_only`, `bus_terminal::services::an_instance_without_the_rights_is_denied_the_server_methods`, `exec::stdin_reaches_the_program_and_is_acknowledged`.

#### Scenario: Input acknowledged
- **WHEN** a client runs `cat` with `stdin` and calls `exec.input {data: "typed\n", eof: true}`
- **THEN** the reply's `taken` is 6 and the stream's stdout is `typed`

#### Scenario: Input overrun
- **WHEN** a client runs a program that reads no stdin and calls `exec.input` with 4 MiB and one byte
- **THEN** the reply is `bad_request`, the stream ends with `reason` `input overrun`, and a later `exec.input` to it, or one from another connection, is `not_found`

#### Scenario: Server command over TCP
- **WHEN** an owner connected on TCP with the access token calls `server.command {argv: ["--version"]}`
- **THEN** the reply is `denied` and nothing runs; the same call on the Unix socket prints the version

### Requirement: Copies in a pane on the bus
A copy made in a pane SHALL emit `pane.copied {pane_id, transfer_id, mime, total_size}` (scope `pane:<id>`), and `clipboard.get {transfer_id}` SHALL stream its content on the bulk lane, `pane_id`, `mime` and `total_size` on the first chunk, until a newer copy replaces it (then `not_found`); a caller the authorizer does not let access the copy's pane SHALL get `not_found` as well.

**Verified by**: `bus_terminal::services::a_copy_in_a_pane_is_an_event_and_its_content_a_stream`, `bus_terminal::services::failpoints::a_copy_of_a_pane_the_caller_may_not_access_is_not_found`, `clipboard::a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it`.

#### Scenario: Copy reaches a subscriber
- **WHEN** a subscriber of `pane.copied` watches while a program prints OSC 52 with `hello`
- **THEN** it receives `pane.copied` with scope `pane:<id>` and `clipboard.get` with its `transfer_id` streams `hello`; after a newer copy the old `transfer_id` is `not_found`

### Requirement: The shared clipboard on the bus
`clipboard.put` SHALL take content in calls of one chunk each sharing a `transfer_id`, refuse more than `clipboard_max_bytes` for that chunk and each later one of the transfer, and answer the `last` chunk once the content is readable by the panes' `xclip` and `wl-paste`.

**Verified by**: `bus_terminal::services::the_shared_clipboard_takes_an_upload_in_chunks`.

#### Scenario: Upload read in a pane
- **WHEN** the owner puts `hello` and `world` as two chunks and a pane then runs `xclip -selection clipboard -o`
- **THEN** the pane shows `helloworld`

### Requirement: Reloading and updating the server by call
`server.reload {}` SHALL reload the server as SIGUSR1 does; its caller SHALL see either the reply or the end of its connection at the reload. `server.update {}` SHALL stream the output of `tilrd update` and end with its exit; both SHALL need the owner or the `control` right `reload`, and be `denied` otherwise.

**Verified by**: `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`, `bus_terminal::services::an_instance_without_the_rights_is_denied_the_server_methods`.

#### Scenario: Reload by call
- **WHEN** the owner calls `server.reload` while a pane runs `sleep 1000`
- **THEN** the reply arrives or the connection ends before it, the connection ends, the server reloads, and after reconnecting `pane.list` shows the pane with the same pid

#### Scenario: An instance without the right
- **WHEN** an instance whose rights let it call `server.*` but name no `control` right calls `server.reload` and `server.update`, and `server.command`
- **THEN** each is `denied` and the server does not reload

#### Scenario: Update by call
- **WHEN** the owner calls `server.update` and the CDN's latest release is the one installed
- **THEN** the stream's stdout says it is the latest release and `End` has code 0

### Requirement: What a connection made is limited and goes with it
A connection SHALL have at most 32 watches, 16 `fs.write` uploads in progress and 64 programs running; one more SHALL be `busy`, and taken again once one of them is gone. Its watches, uploads and programs SHALL go when the connection ends or another connection supersedes it (a program's group killed, an upload's temporary file removed); a call the server takes after its connection ended SHALL keep nothing.

**Verified by**: `bus_terminal::services::a_connection_has_at_most_16_uploads_and_32_watches`, `bus_terminal::services::a_connection_runs_at_most_64_programs`, `bus_terminal::services::what_a_connection_made_goes_when_it_ends_or_is_superseded`, `bus_terminal::services::failpoints::a_first_chunk_taken_after_its_connection_ended_starts_no_upload`.

#### Scenario: One upload too many
- **WHEN** a connection has started 16 uploads with a first chunk each and starts a 17th
- **THEN** the reply is `busy`; once one of them got its `last` chunk, a new one is taken

#### Scenario: Gone with the connection
- **WHEN** a connection with a watch, an upload in progress and a running program whose child holds a FIFO open ends, or is superseded by a connection of the same client
- **THEN** the program's group is killed (the FIFO's writer is gone) and its watch and upload are dropped

#### Scenario: A chunk after the end
- **WHEN** a first `fs.write` chunk reaches the server's files after its connection ended
- **THEN** it is refused, and no temporary file and no note of a write are left
