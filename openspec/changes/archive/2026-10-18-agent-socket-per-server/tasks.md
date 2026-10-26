# Tasks

## 1. A directory per server

- [x] 1.1 `agent::socket_path` is `agent/<pid>/<pane id>.sock`; the sweep removes the directories of pids not running and connects to nothing; verify with `agent::tests::only_the_directories_of_servers_that_are_gone_are_swept` (a live pid's socket that is bound but not listening stays) and `agent::the_sockets_a_killed_server_left_are_removed_when_the_next_starts`
- [x] 1.2 A server starting leaves another live server's sockets alone: no stream opened, no `agent socket: refused a connection`, the same socket file; verify with `agent::a_server_starting_leaves_the_agent_sockets_of_a_live_one_alone` (red before the change)
- [x] 1.3 Stop removes the server's directory; a reload makes the socket again at the path the shell has (the flat one for a pane of a server from before the change); verify with `agent::the_panes_socket_works_after_a_reload` (which now waits for the resume log line: the harness reads the log in a thread, and the assertion right after the reconnect failed once in `scripts/linux.sh`), the legacy path by review
- [x] 1.4 itests UC-005, UC-035, UC-036, UC-069 follow the new path; README "SSH agent forwarding", USE_CASES.md and itests/USE_CASES.md; verify with `./itests/dt` (UC-036 green) and review

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; `cargo test -p tilrd --features tilrd/failpoints --test agent` 10 times in a row green
- [x] 2.2 `./itests/dt` then `./itests/dt coverage` exits 0
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
