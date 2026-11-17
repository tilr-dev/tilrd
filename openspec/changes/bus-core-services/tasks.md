# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former single change `unified-bus` (shared design, O3).

## 1. Services on the bus

- [x] 1.1 [4.2] Protocol-free entry points: `files::Access`, `exec::Execs::start|input|signal|cancel` with `Report`, `clipboard::Receiver::put`; the `tilr.v1` handlers on them; verify every existing `files::*`, `exec::*` and `clipboard::*` test green unchanged
- [x] 1.2 [4.2] `bus/core/server.rs`: `server.info`, `server.reload`, `server.update`, `server.command`; verify `bus_terminal::services::server_info_describes_the_server`, `bus_terminal::services::server_commands_are_taken_on_the_unix_socket_only`, `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`
- [x] 1.3 [4.2] `bus/core/fs.rs`: `fs.*` and `fs.changed.<watch_id>`, only with `files`; verify `bus_terminal::services::files_are_listed_changed_and_watched_on_the_bus`, `bus_terminal::services::file_content_moves_in_chunks`
- [x] 1.4 [4.2] `bus/core/exec.rs`: `exec.run|input|signal`, only with `exec`; verify `bus_terminal::services::a_program_runs_on_the_bus_and_its_outputs_come_apart`, `bus_terminal::services::a_cancelled_program_ends_with_its_group`
- [x] 1.5 [4.2] `bus/core/clipboard.rs`: `clipboard.put|get` and `pane.copied` (hook in `Supervisor::send_copy`), only with `clipboard`; verify `bus_terminal::services::a_copy_in_a_pane_is_an_event_and_its_content_a_stream`, `bus_terminal::services::the_shared_clipboard_takes_an_upload_in_chunks`
- [ ] 1.6 [4.2] `bus/core/host.rs`: `host.open` called on each `pane.output` consumer of the pane that provides it *(blocked: needs a broker call to one connection; design.md, Gaps)*
- [ ] 1.7 [4.2] `bus/core/agent.rs`: `ssh_agent.request|close` called on the consumer that opened its stream last *(blocked as 1.6)*
- [x] 1.8 Listing: verify `bus_terminal::services::tilrd_provides_the_terminal_methods`; README "Services" and the "Panes" corrections; USE_CASES.md entries (own section)

## 2. Checks and archive

- [x] 2.1 [12.1] Run the checks from AGENTS.md in Linux (`scripts/linux.sh`): fmt, both clippy runs, the bus and service tests; on macOS the new tests and `cargo deny check`
- [ ] 2.2 [12.2] Archive the change and list it under Done in `openspec/ROADMAP.md`; verify `openspec validate --all --strict`
