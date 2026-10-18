# Tasks

## 0. Decision

- [ ] 0.1 Owner chooses the scheme; write design.md and the `reload` deltas; drop `skip_specs`; verify with `openspec validate reload-rollback --strict`

## 1. Implementation

- [ ] 1.1 Implement the scheme; verify with the failpoints case "a version that crashes after taking over" turning from shells-lost to shells-kept (`reload::…crashes_after_taking_over…`) and itests UC-066 (rollback and forward with panes)

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
