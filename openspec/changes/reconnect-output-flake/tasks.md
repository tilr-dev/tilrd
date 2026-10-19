# Tasks

## 1. Flake

- [ ] 1.1 (TILR-111) `reconnect::output_printed_while_nobody_watched_is_there_on_reconnect`: find why the output is missing after the reconnect under load (the test's `sleep 1` against the reconnect's timing, or a lost frame); fix with the cause recorded; verify 10 consecutive green Linux full runs

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
