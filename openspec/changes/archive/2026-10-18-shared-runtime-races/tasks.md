# Tasks

## 1. The shared runtime files stay usable while another server starts

- [x] 1.1 `install_shims` leaves a right link alone and replaces a changed one by rename (`bin/.<name>.<pid>.tmp`); verify with `clipboard::tests::the_shims_are_never_missing_while_another_server_installs_them` (red before: the link missing 35045 times in 2000 installs) and `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` (red before: 3 of 3)
- [x] 1.2 `reload::write_shared_file` for the shell wrappers (unchanged: not written; else a temporary file of the write's own); verify with `shell::tests::servers_starting_at_once_each_install_the_wrappers` (red before) and `shell::tests::the_wrappers_are_written_private_and_replaced_whole`
- [x] 1.3 `local::peer_gone`: a connection closed before its uid check is closed with a debug line on the host socket, the Unix socket and the agent sockets; verify with `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` (no `refused a connection` in the first server's log; red before on macOS: 3 lines) and UC-035 (another uid still refused)
- [x] 1.4 README (tools and host socket) and USE_CASES.md; verify by review

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 2.2 `./itests/dt` then `./itests/dt coverage` exits 0
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
