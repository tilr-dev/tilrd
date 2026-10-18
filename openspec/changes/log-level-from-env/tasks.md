# Tasks

## 1. Filter

- [ ] 1.1 Enable the env filter; verify with `reconnect::rust_log_debug_shows_debug_lines` (stderr has a DEBUG line) and default output unchanged
- [ ] 1.2 README "Logging" documents `RUST_LOG`; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
