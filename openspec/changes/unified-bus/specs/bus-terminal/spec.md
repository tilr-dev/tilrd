## ADDED Requirements

### Requirement: The terminal is served on the bus only
The server SHALL serve panes, files, exec, the clipboard, host events and the SSH agent only as methods, events and streams of `tilr.bus.v1`, provided or emitted by the participant `tilrd`; `crates/tilr-proto/proto/tilr.proto` and the package `tilr.v1` MUST NOT exist in the repository, except for the one refusal packet of the `component-bus` spec.

**Verified by**: `bus_terminal::tilrd_provides_the_terminal_methods`, CI step `! test -e crates/tilr-proto` in `.github/workflows/ci.yml`.

#### Scenario: Methods listed
- **WHEN** the owner calls `bus.methods` on a server started with default options
- **THEN** `tilrd` is the provider of `pane.create`, `pane.list`, `pane.close`, `pane.resize`, `pane.input`, `pane.read`, `pane.applied`, `pane.input_finalize`, `pane.profiles`, `pane.output`, `server.info`, `server.reload`, `server.update`, `server.command`, `fs.list`, `fs.stat`, `fs.read`, `fs.write`, `fs.mkdir`, `fs.rename`, `fs.delete`, `fs.watch`, `fs.unwatch`, `exec.run`, `exec.input`, `exec.signal`, `clipboard.put` and `clipboard.get`

### Requirement: server.info describes the server
`server.info {}` SHALL answer `server_version`, `update_channel` (from `server.toml`, default `stable`), `server_id` (a UUID kept in `server-id`), `features`, `clipboard_max_bytes` and `fs_max_bytes` (0 without their feature). `features` SHALL list exactly what this server can be run without and offers, in this order: `clipboard`, `files`, `host-events`, `agent`, `exec`, `sandbox`; the methods of a feature not listed MUST NOT be provided.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `panes::a_server_keeps_its_id_when_it_restarts`, `panes::hello_reports_the_configured_update_channel`, `exec::exec_is_offered_unless_turned_off`, `sandbox::sandbox_is_offered_where_it_can_be_enforced`.

#### Scenario: Everything on
- **WHEN** a server started with default options on a host that can enforce sandboxes is asked `server.info`
- **THEN** `features` is `["clipboard", "files", "host-events", "agent", "exec", "sandbox"]`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec`
- **THEN** `exec` is missing from `features`, and a call to `exec.run` is answered `not_found`

### Requirement: A pane is created, listed and closed by calls
`pane.create {cols, rows, command, cwd, profile}` SHALL start a pane as the `panes` and `pane-profiles` specs say and answer its `PaneInfo` once the program runs; `pane.list {}` SHALL answer the `PaneInfo` of every pane the caller may access; `pane.close {pane_id}` SHALL close it and answer `{exit_code}` once the program ended. A pane id that names no pane SHALL be answered `not_found` with `no such pane: <name as sent>` and the connection MUST stay open.

**Verified by**: `panes::a_pane_runs_commands_and_reports_how_its_shell_exited`, `panes::asking_for_a_pane_that_does_not_exist_gets_an_error`, `bus_terminal::a_pane_is_created_listed_and_closed_by_calls`.

#### Scenario: Create and list
- **WHEN** the owner calls `pane.create {cols: 100, rows: 30}`
- **THEN** the reply's `PaneInfo.pane_id` is a UUID, and `pane.list` lists that pane with `cols` 100 and `rows` 30

#### Scenario: Unknown pane
- **WHEN** the owner calls `pane.close {pane_id: "1"}`
- **THEN** the reply is `not_found` with `no such pane: 1`

### Requirement: Pane changes are events
The server SHALL emit `pane.created` (`PaneInfo`) when a pane appears, `pane.changed` (`PaneInfo`) when its size, title, directory or viewers change, `pane.title {pane_id, title}` when its title changes, `pane.bell` and `pane.command_finished {pane_id, at_ms}` for its activity, `pane.copied` for a copy, and `pane.exited {pane_id, exit_code}` when its program ends, each with scope `pane:<id>`.

**Verified by**: `bus_terminal::pane_changes_are_events`, `panes::a_bell_in_an_unwatched_pane_is_reported`, `panes::a_finished_foreground_command_is_reported`.

#### Scenario: A pane's life
- **WHEN** a subscriber of `pane.*` watches while the owner creates a pane, its shell sets the title to `t`, rings the bell and exits with 3
- **THEN** it receives `pane.created`, `pane.title` with `t`, `pane.bell` and `pane.exited` with `exit_code` 3, in that order

### Requirement: Watching a pane is a stream of frames
`pane.output {pane_id}` SHALL be a stream on the interactive lane whose first chunk is a full frame of the pane's screen and whose later chunks are the frames the pane process builds from its screen model, with the content the `screen-frames` spec gives a `Frame`; it SHALL end with `End{exit_code}` when the program exits. A frame whose encoding would pass `max_frame` SHALL be preceded by chunks carrying its images. The caller SHALL count as a viewer while the stream is open.

**Verified by**: `reconnect::subscribing_again_sends_the_whole_screen_again`, `panes::two_clients_watch_and_type_into_the_same_pane`, `panes::a_client_that_stops_watching_gets_no_more_frames`, `bus_terminal::watching_a_pane_is_a_stream_of_frames`.

#### Scenario: Typed command shows
- **WHEN** a client opens `pane.output` on a new pane and inputs `echo "sum is $((6 * 7))"` and Enter
- **THEN** its chunks show `sum is 42`, and `pane.list` lists it in `viewed_by`

#### Scenario: Stop watching
- **WHEN** the client cancels the stream
- **THEN** no more frames arrive and `pane.list` no longer lists it in `viewed_by`

### Requirement: A lagging watcher skips to the latest screen
A `pane.output` consumer whose connection's interactive lane has held a packet over 250 ms, or whose stream credit is spent, SHALL have its frames of that pane skipped until it caught up and SHALL then receive a full frame whose `last_input_seq` is its latest seq a skipped frame echoed; it MUST NOT be dropped for lagging nor slow other consumers.

**Verified by**: `metrics::a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo`, `reconnect::a_lagging_client_gets_the_last_screen_without_new_output`, itests `UC-049` (`uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others`).

#### Scenario: No credit
- **WHEN** a consumer opens `pane.output` with a 64 KiB window, credits nothing while the pane prints 30 MB, then credits again
- **THEN** it receives a full frame matching the pane's screen and stays connected

### Requirement: Input is acknowledged after it is applied
`pane.input {pane_id, seq, input_seq, key | mouse | paste | raw | scroll | clear}` and `pane.resize {pane_id, cols, rows, cell_width_px, cell_height_px, seq, input_seq}` SHALL be applied in the order a connection sent them and answered once applied (the pty took the last byte, or the scroll, clear or resize took effect); `seq` SHALL be echoed in `Frame.last_input_seq` as the `screen-frames` spec says, with the connection's `seq_base` = `HelloAck.connection_id << 32`.

**Verified by**: `input_delivery::disconnect_before_application_replays_one_copy`, `clear_pane::clearing_history_reaches_all_viewers_and_survives_reconnecting`, `bus_terminal::input_is_acknowledged_after_it_is_applied`.

#### Scenario: A program that prints nothing
- **WHEN** a client inputs `x` into a pane whose program reads without echoing
- **THEN** the call is answered and no frame is needed for it

#### Scenario: Order kept
- **WHEN** a client sends 100 `pane.input` calls with `raw` `1` to `100` without waiting
- **THEN** the program reads them in that order

### Requirement: Numbered input is applied once across reconnects
Input with `input_seq` > 0 SHALL need a `client_id` in `Hello` (else `bad_request: numbered input requires a stable client_id`) and SHALL be applied at most once per `client_id` and pane; `pane.applied {}` SHALL answer the caller's applied counters per running pane within 5 s; input naming a missing pane SHALL get a terminal `not_found` and change no counter. `pane.input_finalize {}` SHALL end the session at once; an unfinalized one SHALL be forgotten an hour after its last connection.

**Verified by**: `input_delivery::applied_input_with_a_lost_ack_is_not_applied_twice`, `input_delivery::numbered_input_requires_a_stable_client_id`, `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect`, `input_delivery::finalized_sessions_do_not_accumulate_delivery_history`, `input_delivery::a_stuck_pane_does_not_block_other_panes_hello`.

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
- **WHEN** a pane process stops reading its input pipe and a client calls `pane.applied`
- **THEN** within 5 s the stuck pane is closed and the answer carries the counters of the other panes

#### Scenario: Internal fields cannot be forged
- **WHEN** a participant sends `pane.input` with fields the supervisor uses internally towards pane processes
- **THEN** no such field exists in `tilr.pane.v1`, and no other identity's delivery state changes

### Requirement: Reading a pane's text
`pane.read {pane_id, since_seq?, wait_ms?, history_lines?}` SHALL answer the pane's screen as text rows with `seq`, size, cursor, title, cwd, `alive` and `exit_code`, and up to `history_lines` (≤ 10000) scrollback lines; with `since_seq` it SHALL wait for a change after that `seq` or the program's exit, at most `min(wait_ms, 25 s)` and always before the call's deadline.

**Verified by**: `bus_terminal::reading_a_pane_and_waiting_for_a_change`.

#### Scenario: Wait for a change
- **WHEN** an instance calls `pane.read {since_seq: S, wait_ms: 5000}` with the current `seq` S and 1 s later a client types into the pane
- **THEN** the answer comes after about 1 s with a `seq` greater than S

#### Scenario: Deadline shorter than the wait
- **WHEN** the call's `timeout_ms` is 500 and `wait_ms` is 5000 and nothing changes
- **THEN** the answer comes before 500 ms with `seq` S, not a `timeout`

### Requirement: Creating a pane on the bus needs pane rights
For a caller other than the owner, `pane.create` SHALL be `denied` unless its token has `pane_profile(<the profile>)` or `pane_profile_any()` (or, without a profile, `pane_unrestricted()`), and, when the request sets `command` or `cwd`, also `pane_override()`; the denial SHALL name the missing right and no pane SHALL start.

**Verified by**: `bus_terminal::creating_a_pane_needs_pane_rights`.

#### Scenario: Another profile
- **WHEN** `t-1` has `pane = ["profile:agent"]` and calls `pane.create {profile: "demo"}`
- **THEN** the reply is `denied` naming `pane_profile`, and no pane is listed

#### Scenario: No profile
- **WHEN** `t-1` with the same rights calls `pane.create {}`
- **THEN** the reply is `denied` naming `pane_unrestricted`

#### Scenario: Override
- **WHEN** `t-1` calls `pane.create {profile: "agent", command: ["/bin/sh"]}`
- **THEN** the reply is `denied` naming `pane_override`

### Requirement: Pane access follows the creator
`PaneInfo.created_by` SHALL be the identity of the participant that created the pane (`owner`, `instance:<name>`, `token:<name>`) and SHALL cross a reload. A caller whose token has `pane_access("own")`, or names no `pane_access`, SHALL list, watch, read, input to, resize and close only panes whose `created_by` is its identity, and SHALL receive pane events only for them; with `pane_access("any")` every pane; any other access MUST be `denied`.

**Verified by**: `bus_terminal::pane_access_follows_the_creator`, `reload::a_bus_created_pane_keeps_its_creator_across_a_reload`.

#### Scenario: The owner's pane
- **WHEN** the owner created a pane and `t-1` (no `pane_access`) calls `pane.read` on it and subscribes to `pane.*`
- **THEN** the read is `denied`, `pane.list` from `t-1` does not include it, and `t-1` receives no event of it

#### Scenario: Across a reload
- **WHEN** `t-1` created a pane, the server reloads, and `t-1` inputs to it
- **THEN** the input is applied and `pane.list` shows `created_by` `instance:t-1`

### Requirement: A pane outlives the participant that made it
A pane SHALL keep running when the participant that created it disconnects, stops, crashes or is reloaded; that participant's next connection SHALL access it under the same rights.

**Verified by**: `bus_terminal::a_pane_outlives_the_participant_that_made_it`.

#### Scenario: Instance reloaded
- **WHEN** `t-1` created a pane running `sleep 1000` and `tilr bus reload t-1` runs
- **THEN** the pane still runs and the new `t-1` process can call `pane.read` on it

### Requirement: Files on the bus
`fs.list`, `fs.stat`, `fs.mkdir`, `fs.rename`, `fs.delete`, `fs.watch` and `fs.unwatch` SHALL be calls answering what `FsEntries`, `FsInfo` and `FsAck` answered; paths, roots, read-only mode and the `*_bytes` twins SHALL behave as the `files` spec says; a watch SHALL emit `fs.changed.<watch_id>` events until `fs.unwatch` or the end of the caller's connection.

**Verified by**: `files::a_client_lists_a_directory_and_looks_at_its_entries`, `files::directories_are_made_entries_renamed_and_deleted`, `files::a_watched_directory_reports_what_changes_in_it`.

#### Scenario: Watch a directory
- **WHEN** a client calls `fs.watch` on a directory, subscribes to `fs.changed.<watch_id>`, and a file is created there
- **THEN** it receives an `fs.changed.<watch_id>` event naming the file with kind create

### Requirement: File content moves in chunks
`fs.read` SHALL be a stream on the bulk lane of chunks of at most 256 KiB that ends with an error when the read fails; `fs.write` SHALL take the content in calls of one chunk each sharing a `transfer_id` and replace the file only after the `last` chunk.

**Verified by**: `files::a_file_is_read_in_chunks_whole_or_in_part`, `files::a_file_is_written_whole_or_not_at_all`, `files::a_client_answering_only_the_latest_ping_gets_the_whole_file`.

#### Scenario: Read a large file slowly
- **WHEN** a client reads a 16 MiB file with `fs.read` and a 256 KiB window, crediting slowly, while it types into a pane
- **THEN** the whole file arrives intact and its keystrokes' frames are not held behind the file

### Requirement: Exec on the bus
`exec.run {argv, cwd, env, stdin, timeout_ms}` SHALL be a stream on the bulk lane whose first chunk is `started {exec_id, pid}` (or an error reply if it did not start), then `output {stream, data}` chunks in order per stream, and `End` with the exit (`code`, `signal`, `timed_out`, `cancelled`, `reason`); cancelling the stream SHALL kill the program's group; a reader that credits slowly SHALL hold the program back.

**Verified by**: `exec::a_program_runs_without_a_shell_and_its_outputs_come_apart`, `exec::a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client`, `exec::a_cancelled_program_ends_at_once_with_its_children`.

#### Scenario: Cancel
- **WHEN** a client runs `exec.run {argv: ["sleep", "100"]}` and cancels the stream
- **THEN** the program's process group is killed

### Requirement: Exec input, signals and server commands
`exec.input {exec_id, data, eof}` SHALL be answered with `{taken}` once the program took the bytes, and more than 4 MiB sent and not yet answered SHALL kill it (`reason` `input overrun`); `exec.signal {exec_id, signal}` SHALL signal its group; `server.command {argv}` SHALL run the server's own executable as `exec.run` does, for the owner on the Unix socket only.

**Verified by**: `exec::stdin_reaches_the_program_and_is_acknowledged`, `exec::the_signal_that_killed_a_program_is_reported`, `bus_terminal::server_commands_are_taken_on_the_unix_socket_only`.

#### Scenario: Server command over TCP
- **WHEN** an owner connected on TCP with the access token calls `server.command {argv: ["token"]}`
- **THEN** the reply is `denied` and nothing runs

### Requirement: Copies and the shared clipboard on the bus
A copy made in a pane SHALL emit `pane.copied {pane_id, transfer_id, mime, total_size}` (scope `pane:<id>`), and `clipboard.get {transfer_id}` SHALL stream its content on the bulk lane until a newer copy replaces it (then `not_found`). `clipboard.put` SHALL take content in calls of one chunk each sharing a `transfer_id`, refuse more than `clipboard_max_bytes`, and answer the `last` chunk once the content is readable by the panes' `xclip` and `wl-paste`.

**Verified by**: `clipboard::a_pasted_image_reaches_a_program_that_reads_the_clipboard`, `clipboard::xclip_in_a_pane_reaches_the_watching_client`, `clipboard::a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it`.

#### Scenario: Copy reaches a watcher
- **WHEN** a subscriber of `pane.copied` that may access the pane watches while a program prints OSC 52 with `hello`
- **THEN** it receives `pane.copied` and `clipboard.get` with its `transfer_id` streams `hello`

### Requirement: Host events are calls the client serves
For `tilr edit` and `tilr open` in a pane, the server SHALL call `host.open {pane_id, name, kind, path, args}` on each `pane.output` consumer of that pane that provides `host.open`, and SHALL tell the program no client is attached when there is none.

**Verified by**: `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file`, `files::tilr_edit_fails_with_no_client_watching_or_outside_a_pane`.

#### Scenario: Nobody serves host.open
- **WHEN** a client watches a pane without providing `host.open` and `tilr edit notes.md` runs in it
- **THEN** the client is not called and `tilr edit` says no client is attached

### Requirement: The SSH agent is a call the client serves
A program connecting to a pane's `SSH_AUTH_SOCK` SHALL have each agent message forwarded as `ssh_agent.request {conn_id, pane_id, message}` to the `pane.output` consumer of that pane that provides it and opened its stream last, and the reply written back to the program; `ssh_agent.close {conn_id}` SHALL be called when the program closes; with no such consumer the program's connection SHALL be closed at once.

**Verified by**: `agent::a_program_talks_to_the_agent_of_the_client_through_the_panes_socket`.

#### Scenario: Agent forwarded
- **WHEN** a client provides `ssh_agent.request`, watches a pane, and `ssh-add -l` runs in it
- **THEN** the client receives the request and the program prints the keys the client answered

### Requirement: Reloading and updating the server by call
`server.reload {}` SHALL reload the server as SIGUSR1 does and answer before the exec; `server.update {}` SHALL stream the output of `tilrd update` and end with its exit; both SHALL need the owner or the `control` right `reload`.

**Verified by**: `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`.

#### Scenario: Reload by call
- **WHEN** the owner calls `server.reload` while a pane runs `sleep 1000`
- **THEN** the reply arrives, the connection ends, and after reconnecting `pane.list` shows the pane with the same pid
