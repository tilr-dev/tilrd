# Tasks

## 1. Platforms

- [ ] 1.1 Try a musl build; verify `tilrd --version` in an Alpine container and itests group A on Alpine
- [x] 1.2 Publish Developer ID signed and notarized macOS releases on the CDN; verify `tilrd update` on macOS from the per-platform layout

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
