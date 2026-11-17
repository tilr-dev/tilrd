## ADDED Requirements

### Requirement: server.info describes the server
`server.info {}` SHALL answer `server_version`, `update_channel` (from `server.toml`, default `stable`), `server_id` (a UUID kept in `server-id`), `features`, `clipboard_max_bytes` and `fs_max_bytes` (0 without their feature). `features` SHALL list exactly what this server can be run without and offers, in this order: `clipboard`, `files`, `host-events`, `agent`, `exec`, `sandbox`; the methods of a feature not listed MUST NOT be provided.

**Verified by**: `panes::hello_announces_the_clipboard_and_its_limit`, `panes::a_server_keeps_its_id_when_it_restarts`, `panes::hello_reports_the_configured_update_channel`, `exec::exec_is_offered_unless_turned_off`, `sandbox::sandbox_is_offered_where_it_can_be_enforced`.

#### Scenario: Everything on
- **WHEN** a server started with default options on a host that can enforce sandboxes is asked `server.info`
- **THEN** `features` is `["clipboard", "files", "host-events", "agent", "exec", "sandbox"]`

#### Scenario: A feature turned off
- **WHEN** the server runs with `--no-exec`
- **THEN** `exec` is missing from `features`, and a call to `exec.run` is answered `not_found`

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
