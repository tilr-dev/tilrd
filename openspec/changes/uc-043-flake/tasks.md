# Tasks

## 1. Flake

- [ ] 1.1 itests `uc_043`: find and fix the flake or quarantine it with its cause in itests/USE_CASES.md; verify 10 consecutive green runs of `./itests/dt` UC-043

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
