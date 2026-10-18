# Tasks

## 0. Decision

- [ ] 0.1 Owner decides the placement; write design.md and the `directory-sync` spec; drop `skip_specs`; verify with `openspec validate directory-sync --strict`

## 1. Implementation

- [ ] 1.1 Per the design; verify with the group S itests cases

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
