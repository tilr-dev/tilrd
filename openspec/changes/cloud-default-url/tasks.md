# Tasks

## 1. Default

- [ ] 1.1 Confirm the production host (curl `/v1/enroll` on both); change the default; verify with the unit test `the_control_address_follows_the_cloud_address` updated and `tilrd cloud login --help`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
