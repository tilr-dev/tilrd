# Tasks

## 1. Parent death and process group

- [ ] 1.1 Parent-death signal and own process group; verify with itests UC-088 `uc_088.py::cloudflared_is_gone_after_the_server_is_sigkilled` (fake cloudflared)
- [ ] 1.2 A terminal hangup does not touch cloudflared; verify with an itests UC-013 variant with a tunnel up

## 2. Stuck cloudflared at reload

- [ ] 2.1 SIGKILL after the wait and exec anyway; verify with a failpoints test with a cloudflared stub that ignores SIGTERM (`cloud::a_stuck_cloudflared_does_not_hold_the_reload`)
- [ ] 2.2 Update the `cloud-tunnels` spec Notes (UC-088 no longer failing); USE_CASES.md entries; verify by review against this proposal

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
