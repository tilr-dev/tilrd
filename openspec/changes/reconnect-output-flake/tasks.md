# Tasks

## 1. Flake

- [x] 1.1 `reconnect::output_printed_while_nobody_watched_is_there_on_reconnect`: establish and record the cause of missing output; reproduce the failing event order deterministically using reconnect failpoints or controlled socket reads, without sleeps, retries, or increased timeouts; fix the cause and verify the regression fails before the fix and passes after it.

## 2. Checks

- [x] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
