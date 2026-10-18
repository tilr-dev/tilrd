# Tasks

## 1. Per-process path

- [ ] 1.1 Name the socket per process and sweep dead ones; verify with `files::two_servers_of_one_user_keep_their_host_sockets` (both `tilr edit` work)
- [ ] 1.2 Keep the old path for panes from before an update; verify with itests UC-076 after UC-060
- [ ] 1.3 README "Host socket" updated; USE_CASES.md entry; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
