# Tasks

Deferred: decided when the project is complete. The checklist is kept for that moment.

## 1. Readiness

- [ ] 1.1 Parametrize the Dagger engine and hosts in itests; verify `grep -rn 'pb\.rexuni\|dagger-pb' itests/` is empty
- [ ] 1.2 Fix README links; verify every relative link resolves inside the repository
- [ ] 1.3 Read through README, TODO.md and USE_CASES.md for private names and stale claims; verify by review and `gitleaks git --redact --no-banner --exit-code 1 .`
- [ ] 1.4 Topics, private vulnerability reporting, `RELEASE_LINUX_ARM64=true` after the switch; verify the next release has `linux-aarch64`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
