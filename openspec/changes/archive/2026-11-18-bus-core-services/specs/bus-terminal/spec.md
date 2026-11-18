## ADDED Requirements

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

**Verified by**: `bus_terminal::services::a_connection_has_at_most_16_uploads_and_32_watches`, `bus_terminal::services::a_failed_upload_is_no_longer_in_progress`, `bus_terminal::services::a_connection_runs_at_most_64_programs`, `bus_terminal::services::what_a_connection_made_goes_when_it_ends_or_is_superseded`, `bus_terminal::services::failpoints::a_first_chunk_taken_after_its_connection_ended_starts_no_upload`.

#### Scenario: One upload too many
- **WHEN** a connection has started 16 uploads with a first chunk each and starts a 17th
- **THEN** the reply is `busy`; once one of them got its `last` chunk, a new one is taken

#### Scenario: A failed upload is not in progress
- **WHEN** 16 uploads of a connection failed on their first chunk, before their `last` one, and it starts another
- **THEN** the new one is taken; a later chunk of a failed one gets that failure, and a first chunk with a path starts its transfer afresh

#### Scenario: Gone with the connection
- **WHEN** a connection with a watch, an upload in progress and a running program whose child holds a FIFO open ends, or is superseded by a connection of the same client
- **THEN** the program's group is killed (the FIFO's writer is gone) and its watch and upload are dropped

#### Scenario: A chunk after the end
- **WHEN** a first `fs.write` chunk reaches the server's files after its connection ended
- **THEN** it is refused, and no temporary file and no note of a write are left
