# Tasks

## 0. Decision

- [ ] 0.1 Owner chooses flags or a version bump; write the `protocol` / `files` deltas and drop `skip_specs`; verify with `openspec validate protocol-feature-flags --strict`

## 1. Implementation

- [ ] 1.1 Advertise the chosen flags (or bump); verify with `protocol::hello_ack_lists_the_features` updated and itests UC-071 using the flag
- [ ] 1.2 README "Protocol" lists every flag and what it promises; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
