# Tasks

## 1. Leftover servers

- [ ] 1.1 Find what leaves `target/debug` test servers running; verify `pgrep -f target/debug/tilrd` is empty after `cargo test --workspace --features tilrd/failpoints --locked`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
