# Tasks

## 1. Signals across the exec

- [ ] 1.1 Block SIGUSR1 and SIGHUP before exec (only into a successor that prints `reload-check 1`), unblock after handlers are installed; verify with a failpoints test sending SIGUSR1 and SIGHUP at `supervisor-resume-live=sleep:500` (server alive, one more reload done)

## 2. Socket after a refused reload

- [ ] 2.1 Keep the socket and lock serving after a refused or failed reload; verify with `reload::a_refused_reload_keeps_the_socket` (refusal by `supervisor-resume=panic` in the check) and an itests UC-012 style check that `tilr ls` starts no second daemon
- [ ] 2.2 README "Reloading without closing shells" and USE_CASES.md entries for the new tests; verify by review against this proposal

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
