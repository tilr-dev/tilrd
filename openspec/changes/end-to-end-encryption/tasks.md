# Tasks

## 0. Decision

- [ ] 0.1 Owner provides or approves the design; write design.md and the deltas; drop `skip_specs`; verify with `openspec validate end-to-end-encryption --strict`

## 1. Implementation

- [ ] 1.1 Per the design; verify with a cargo test where a proxy sees no plaintext and itests UC-07A through an encrypted tunnel

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
