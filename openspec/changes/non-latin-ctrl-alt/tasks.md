# Tasks

## 0. Decision

- [ ] 0.1 Owner picks the policy; write the `screen-frames` deltas and drop `skip_specs`; verify `openspec validate non-latin-ctrl-alt --strict`

## 1. Implementation

- [ ] 1.1 Encode per the chosen policy; verify with cargo tests in `crates/tilr-screen` for Ctrl+С (ru), Option+c, AltGr+e, Alt+Shift+1 and itests UC-079 extended

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
