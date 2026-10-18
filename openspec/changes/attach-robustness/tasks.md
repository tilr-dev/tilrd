# Tasks

## 1. Attach fixes

- [ ] 1.1 Retry list/subscribe errors after reconnect; verify with `client::attach_survives_an_error_after_a_reconnect`
- [ ] 1.2 Immediate detach while connecting; verify with `client::detach_works_while_connecting`
- [ ] 1.3 Restore the terminal on SIGTERM; verify with an itests case in group B (`stty -a` after `kill -TERM` shows cooked mode)

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
