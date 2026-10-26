# Tasks

## 1. Path choice and removal

- [x] 1.1 `host::choose_path` (own `<server_id>-<pid>.sock` if present, else `<server_id>.sock` unless a connect is accepted, else `<server_id>-<pid>.sock`) and sweep of dead-pid sockets; verify with unit tests in `host.rs` (`host::tests::a_live_server_at_the_shared_path_moves_the_next_one`, `host::tests::a_stale_socket_is_taken_over`, `host::tests::sockets_of_dead_pids_are_swept`)
- [x] 1.2 Remove on stop only when `(dev, ino)` still match what was bound; verify with the new `files::two_servers_of_one_user_keep_their_host_sockets` (two servers, one state directory: `tilr edit` in a pane of each reaches its own client; stop one, the other's still works)
- [x] 1.3 Reload keeps the chosen path; verify with `reload::a_second_server_keeps_its_host_socket_across_a_reload` (second server reloads with SIGUSR1; a pane created before runs `tilr edit` successfully)
- [x] 1.4 README "Host socket" names both paths; USE_CASES.md entries; verify by review against this proposal

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 2.2 `./itests/dt --only scenarios --filter uc_035` green (no case added: two servers on one machine are covered by the cargo tests), then `./itests/dt coverage` exits 0
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
