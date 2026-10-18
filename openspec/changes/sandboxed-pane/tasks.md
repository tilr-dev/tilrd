# Tasks

## 0. Decision

- [ ] 0.1 Owner chooses the macOS mechanism; write design.md and deltas; drop `skip_specs`; verify with `openspec validate sandboxed-pane --strict`

## 1. Implementation

- [ ] 1.1 Apply the profile; verify with a test that a sandboxed pane cannot write outside its allowed directory on macOS and Linux

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
