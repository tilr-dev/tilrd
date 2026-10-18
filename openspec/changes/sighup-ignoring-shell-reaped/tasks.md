# Tasks

## 1. Kill after grace

- [ ] 1.1 SIGHUP, wait, SIGKILL the pane's group on stop; verify with `panes::a_shell_ignoring_sighup_is_ended_when_the_server_stops` and itests `uc_086.py::a_shell_ignoring_sighup_does_not_outlive_the_server`
- [ ] 1.2 Check that a reload is unaffected: itests UC-061 green; verify by review against this proposal
- [ ] 1.3 Update the `daemon-lifecycle` Notes and USE_CASES.md; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
