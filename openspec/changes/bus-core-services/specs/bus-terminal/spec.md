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

**Verified by**: `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`.

#### Scenario: Outside the roots
- **WHEN** a client calls `fs.list {path: "/"}` on a server that shares its home directory
- **THEN** the reply is `bad_request` with a message saying the path is outside the directories the server shares

### Requirement: Watching files on the bus
`fs.watch {path}` SHALL answer `{watch_id}` and emit `fs.changed.<watch_id>` events until `fs.unwatch {watch_id}` or the end of the caller's connection; `fs.unwatch` of a watch that is not the caller's, or of none, SHALL be `not_found`.

**Verified by**: `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `files::a_watched_directory_reports_what_changes_in_it`.

#### Scenario: Watch a directory
- **WHEN** a client calls `fs.watch` on a directory, subscribes to `fs.changed.*`, and a file is written there
- **THEN** it receives an `fs.changed.<watch_id>` event naming the file with kind create

#### Scenario: Unwatched twice
- **WHEN** a client calls `fs.unwatch` with its watch's id twice
- **THEN** the first reply is `FsAck` and the second `not_found`

### Requirement: File content moves in chunks
`fs.read` SHALL be a stream on the bulk lane of `FsData` chunks of at most 256 KiB and at most the stream's window, ending with an error when the read fails; `fs.write` SHALL take the content in calls of one chunk each sharing a `transfer_id`, replace the file only after the `last` chunk, and answer each chunk of a failed transfer with its error.

**Verified by**: `bus_terminal::services::file_content_moves_in_chunks`, `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `files::a_file_is_written_whole_or_not_at_all`.

#### Scenario: Read a large file in a small window
- **WHEN** a client reads a 4 MiB file with `fs.read` and a 256 KiB window
- **THEN** no chunk is larger than the window and the whole file arrives intact

#### Scenario: Written after the last chunk
- **WHEN** a client sends the first of two `fs.write` chunks of a new file
- **THEN** the file is not there until the second, `last`, chunk is answered

### Requirement: Exec on the bus
`exec.run {argv, cwd, env, stdin, timeout_ms}` SHALL be a stream on the bulk lane whose first chunk is `started {exec_id, pid}` (or an error if it did not start: `spawn_failed` for a program that is not there), then `output {stream, data}` chunks in order per stream, and `End` with the exit (`code`, `signal`, `timed_out`, `cancelled`, `reason`); cancelling the stream or ending the connection SHALL kill the program's group; a reader that credits slowly SHALL hold the program back.

**Verified by**: `bus_terminal::services::a_program_runs_on_the_bus_and_its_outputs_come_apart`, `bus_terminal::services::a_cancelled_program_ends_with_its_group`, `exec::a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client`.

#### Scenario: Outputs apart
- **WHEN** a client runs `exec.run {argv: ["/bin/sh", "-c", "echo out; echo err >&2; exit 3"]}`
- **THEN** it receives `started`, `out` on stream 1, `err` on stream 2, and `End` with code 3

#### Scenario: Cancel
- **WHEN** a client runs a program whose child holds a FIFO open and cancels the stream
- **THEN** the program's process group is killed and the FIFO's writer is gone

### Requirement: Exec input, signals and server commands
`exec.input {exec_id, data, eof}` SHALL be answered with `{exec_id, taken}` once the program took the bytes, and more than 4 MiB sent and not yet answered SHALL kill it (`reason` `input overrun`); `exec.signal {exec_id, signal}` SHALL signal its group; a program of another connection SHALL be `not_found`. `server.command {argv}` SHALL run the server's own executable as `exec.run` does, for the owner on the Unix socket only, and be `denied` otherwise.

**Verified by**: `bus_terminal::services::a_program_runs_on_the_bus_and_its_outputs_come_apart`, `bus_terminal::services::server_commands_are_taken_on_the_unix_socket_only`, `exec::stdin_reaches_the_program_and_is_acknowledged`.

#### Scenario: Input acknowledged
- **WHEN** a client runs `cat` with `stdin` and calls `exec.input {data: "typed\n", eof: true}`
- **THEN** the reply's `taken` is 6 and the stream's stdout is `typed`

#### Scenario: Server command over TCP
- **WHEN** an owner connected on TCP with the access token calls `server.command {argv: ["--version"]}`
- **THEN** the reply is `denied` and nothing runs; the same call on the Unix socket prints the version

### Requirement: Copies in a pane on the bus
A copy made in a pane SHALL emit `pane.copied {pane_id, transfer_id, mime, total_size}` (scope `pane:<id>`), and `clipboard.get {transfer_id}` SHALL stream its content on the bulk lane, `pane_id`, `mime` and `total_size` on the first chunk, until a newer copy replaces it (then `not_found`).

**Verified by**: `bus_terminal::services::a_copy_in_a_pane_is_an_event_and_its_content_a_stream`, `clipboard::a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it`.

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
`server.reload {}` SHALL reload the server as SIGUSR1 does and answer before the exec; `server.update {}` SHALL stream the output of `tilrd update` and end with its exit; both SHALL need the owner or the `control` right `reload`.

**Verified by**: `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`.

#### Scenario: Reload by call
- **WHEN** the owner calls `server.reload` while a pane runs `sleep 1000`
- **THEN** the reply arrives, the connection ends, and after reconnecting `pane.list` shows the pane with the same pid

#### Scenario: Update by call
- **WHEN** the owner calls `server.update` and the CDN's latest release is the one installed
- **THEN** the stream's stdout says it is the latest release and `End` has code 0
