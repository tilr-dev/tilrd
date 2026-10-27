# Tasks

Deferred: security design to be done.

## 1. Security design

- [ ] 1.1 Find Dan's earlier E2E thinking (the note that says it is worked out) or confirm there is none; verify by a Basic Memory link in proposal.md
- [ ] 1.2 Write design.md: threat model, candidate approaches compared, key distribution and pinning, reload and reconnect; verify by owner review
- [ ] 1.3 Owner decides; open the implementation change with deltas; verify `openspec validate --all --strict`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
