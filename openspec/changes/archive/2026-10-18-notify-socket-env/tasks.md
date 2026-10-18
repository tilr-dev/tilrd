# Tasks

## 1. Scrub the variables

- [x] 1.1 One list of removed variables in `pty.rs` (the `XPC_*` ones plus the service-manager ones), applied to pane programs; verify with the new `panes::a_pane_does_not_get_the_service_managers_variables` (server started with `NOTIFY_SOCKET`, `LISTEN_FDS`, `WATCHDOG_USEC` set; `env` in a pane lacks them)
- [x] 1.2 Exec removes them before applying `ExecStart.env`; verify with the new `exec::an_exec_does_not_get_the_service_managers_variables`
- [x] 1.3 itests UC-020: in `service_install_writes_a_notify_user_unit_and_enables_it`, a pane of the systemd-started tilrd runs `env` and lacks `NOTIFY_SOCKET`; add tag `panes/user-variables-of-a-pane`; verify `./itests/dt --only scenarios --filter uc_020`
- [x] 1.4 README pane environment lists the removed variables; USE_CASES.md entries; verify by review against this proposal

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 2.2 `./itests/dt --only scenarios --filter uc_020` green, then `./itests/dt coverage` exits 0
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
