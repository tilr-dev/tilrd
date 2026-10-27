# Tasks

Client-side work; no tilrd change.

## 1. Hand-off

- [ ] 1.1 Confirm `HelloAck.server_id` is enough for clients to tell machines apart; verify with `panes::a_server_keeps_its_id_when_it_restarts` green and the README "Protocol" describing `server_id`
- [ ] 1.2 Record the decision for the apps (Basic Memory note on joining machines) and, if wanted, open a `tilr` multi-machine change; verify by the note link in proposal.md

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
