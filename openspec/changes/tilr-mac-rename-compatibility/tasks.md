# Tasks

Groups 2 (desktop-mac) and 3 (release) moved to TILR-187 (owner's decision, 2026-10-20); the numbering of group 4 is kept.

## 1. The contract in this repository

- [x] 1.1 Check the requirement against the cited tests (the version line is asserted by `panes::ps_lists_the_panes_of_a_running_server`, not `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` as first written; `auth::the_token_is_kept_for_the_user_only_and_across_restarts` reads `token`; `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` reads `state-formats`); add `panes::ps_server_version_fails_when_nothing_listens` for the second scenario; verify `cargo test -p tilrd --test panes`
- [x] 1.2 The `--help` comments of `ps`, `token` and `state-formats` in `crates/tilrd/src/main.rs` say "kept for the Tilr app; see the daemon-lifecycle spec"; verify by review

## 4. Checks and archive

- [ ] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 4.2 No itests case is touched; `./itests/dt coverage` exits 0
- [ ] 4.3 Record this repository's commit in proposal.md and archive the change
