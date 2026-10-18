# Tasks

## 0. Design

- [ ] 0.1 Write design.md (library, framing, migration of connections, reload of the UDP socket) and the deltas; drop `skip_specs`; verify with `openspec validate quic-transport --strict`

## 1. Implementation

- [ ] 1.1 QUIC listener and client support in tilr-driver; verify with UC-043 and UC-051 over QUIC (p50 echo under 1 s at 30% loss)

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
