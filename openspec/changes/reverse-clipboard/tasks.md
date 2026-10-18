# Tasks

## 0. Design

- [ ] 0.1 Write design.md and the delta; drop `skip_specs`; verify with `openspec validate reverse-clipboard --strict`

## 1. Implementation

- [ ] 1.1 Shims and message; verify with `clipboard::xclip_in_a_pane_reaches_the_watching_client`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
