# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. The rest of the core on the bus

- [ ] 1.1 [4.3] `server.info|reload|update|command`, `fs.*` (calls, `fs.read` stream, chunked `fs.write`, watches as events), `exec.run|input|signal`, `clipboard.put|get` and `pane.copied`, calls to `host.open` and `ssh_agent.request|close` (T2, T6); features turned off = methods not provided; verify `bus_terminal::tilrd_provides_the_terminal_methods` (the methods of this change and of `bus-core-panes`; `pane-profiles` adds `pane.profiles` to it), `bus_terminal::server_commands_are_taken_on_the_unix_socket_only`, `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`
- [ ] 1.2 Bus smoke tests of the design: verify `bus_terminal::files_exec_and_the_clipboard_answer_on_the_bus`, `bus_terminal::host_open_and_the_agent_are_called_on_a_watcher`, and every existing `tilr.v1` test of `files`, `exec`, `clipboard`, `agent`, `update` green unchanged; USE_CASES.md entries (own section)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
