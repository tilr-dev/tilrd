# Tasks

## 0. Decision

- [ ] 0.1 Owner confirms a use; write the `access-control` delta; drop `skip_specs`; verify with `openspec validate read-only-viewer --strict`

## 1. Checks

- [ ] 1.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
