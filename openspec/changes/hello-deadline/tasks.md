# Tasks

## 1. Hello deadline and cap

- [ ] 1.1 Close un-greeted connections after 10 s; verify with `auth::a_connection_that_never_says_hello_is_closed_after_10_s` and update uc_039's case name and expectation
- [ ] 1.2 Cap un-greeted connections; verify with itests UC-038 (1000 silent connections: greeted clients keep working, memory flat)
- [ ] 1.3 README "Connections" documents both; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
