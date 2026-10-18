# Tasks

## 1. Flags and configuration in the pre-exec check

- [ ] 1.1 Extend `state-formats` with the accepted flags; refuse a successor missing one in use; verify with a failpoints test `reload::a_successor_that_rejects_the_command_line_is_not_reloaded_into` (a stub executable without `--socket`, pid and shells kept, log line names the flag)
- [ ] 1.2 Run the successor's configuration check before exec; verify with `reload::a_successor_that_refuses_the_configuration_is_not_reloaded_into`
- [ ] 1.3 Treat an old `state-formats` output by the flag list of that release; verify with an itests rollback (as UC-066) to the previous archive release started by `tilr` (all panes alive)

## 2. Signals across the exec

- [ ] 2.1 Block SIGUSR1 and SIGHUP before exec, unblock after handlers are installed; verify with a failpoints test sending SIGUSR1 and SIGHUP at `supervisor-resume` (server alive, one reload done)

## 3. Socket after a refused reload

- [ ] 3.1 Keep the socket and lock serving after a refused or failed reload; verify with `reload::a_refused_reload_keeps_the_socket` and itests UC-012 style check that `tilr ls` starts no second daemon
- [ ] 3.2 README "Reload" and USE_CASES.md entries for the new tests; verify by review against this proposal

## 4. Checks

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
