# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former single change `unified-bus` (shared design, O3).

## 1. Services on the bus

- [x] 1.1 [4.2] Protocol-free entry points: `files::Access`, `exec::Execs::start|input|signal|cancel` with `Report`, `clipboard::Receiver::put`; the `tilr.v1` handlers on them; verify every existing `files::*`, `exec::*` and `clipboard::*` test green unchanged
- [x] 1.2 [4.2] `bus/core/server.rs`: `server.info`, `server.reload`, `server.update`, `server.command`; verify `bus_terminal::services::server_info_describes_the_server`, `bus_terminal::services::server_commands_are_taken_on_the_unix_socket_only`, `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`, `bus_terminal::services::an_instance_without_the_rights_is_denied_the_server_methods`
- [x] 1.3 [4.2] `bus/core/fs.rs`: `fs.*` and `fs.changed.<watch_id>`, only with `files`; verify `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `bus_terminal::services::file_content_moves_in_chunks`, `bus_terminal::services::a_read_only_server_refuses_changes_on_the_bus`, `bus_terminal::services::a_path_out_of_the_roots_is_refused_on_the_bus`, `bus_terminal::services::an_upload_is_held_to_its_size_and_the_servers_limit`, `bus_terminal::services::a_watch_is_unwatched_only_by_its_connection`
- [x] 1.4 [4.2] `bus/core/exec.rs`: `exec.run|input|signal`, only with `exec`; verify `bus_terminal::services::a_program_runs_on_the_bus_and_its_outputs_come_apart`, `bus_terminal::services::a_cancelled_program_ends_with_its_group`, `bus_terminal::services::a_reader_that_credits_slowly_holds_the_program_back`, `bus_terminal::services::exec_input_is_the_callers_and_overrun_kills_the_program`, `bus_terminal::services::failpoints::a_program_cancelled_as_it_starts_ends_with_its_group`
- [x] 1.5 [4.2] `bus/core/clipboard.rs`: `clipboard.put|get` and `pane.copied` (hook in `Supervisor::send_copy`), only with `clipboard`; verify `bus_terminal::services::a_copy_in_a_pane_is_an_event_and_its_content_a_stream`, `bus_terminal::services::the_shared_clipboard_takes_an_upload_in_chunks`, `bus_terminal::services::failpoints::a_copy_of_a_pane_the_caller_may_not_access_is_not_found`
- [x] 1.6 Listing: verify `bus_terminal::services::tilrd_provides_the_terminal_methods`; README "Services" and the "Panes" corrections; USE_CASES.md entries (own section)
- [x] 1.7 What a connection made, per connection: at most 32 watches, 16 uploads in progress and 64 programs (one more `busy`); a call whose connection ended keeps nothing; all of it goes when the connection ends or is superseded; verify `bus_terminal::services::a_connection_has_at_most_16_uploads_and_32_watches`, `bus_terminal::services::a_connection_runs_at_most_64_programs`, `bus_terminal::services::what_a_connection_made_goes_when_it_ends_or_is_superseded`, `bus_terminal::services::failpoints::a_first_chunk_taken_after_its_connection_ended_starts_no_upload`

## 2. Checks and archive

- [x] 2.1 [12.1] Run the checks from AGENTS.md in Linux (`scripts/linux.sh`): fmt, both clippy runs, the bus and service tests; on macOS the new tests and `cargo deny check`
- [x] 2.2 [12.2] Archive the change and list it under Done in `openspec/ROADMAP.md`; verify `openspec validate --all --strict`
