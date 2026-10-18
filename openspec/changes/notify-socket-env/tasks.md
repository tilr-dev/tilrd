# Tasks

## 1. Scrub the variables

- [ ] 1.1 Remove the variables from pane and exec environments; verify with `panes::a_pane_does_not_get_notify_socket` and `exec::an_exec_does_not_get_notify_socket` (server started with `NOTIFY_SOCKET` set)
- [ ] 1.2 Verify with itests UC-020 (`ExecStart` with `$NOTIFY_SOCKET`: `env` in a pane lacks it)

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
