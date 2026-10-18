# Tasks

## 1. Ending the pane's groups

- [ ] 1.1 Foreground group of the master (`tcgetpgrp`) in `pty.rs`; a pane routine that SIGHUPs the shell's and the foreground group, waits up to `KILL_GRACE` and SIGKILLs both (design D2); `ClosePane` uses it; verify with `panes::closing_a_pane_ends_a_foreground_program_ignoring_sighup` and `panes::closing_a_pane_ends_its_shell` still green
- [ ] 1.2 The stop path (`FromSupervisor::Closed` / `None`, `pane.rs:381-383`) runs the routine before the last frame and exit; verify with `panes::a_shell_ignoring_sighup_is_ended_when_the_server_stops`, `panes::a_foreground_program_ignoring_sighup_is_ended_when_the_server_stops`, `panes::a_nohup_background_job_is_left_when_the_server_stops`, and `reload::stopping_the_server_ends_the_shells` still green
- [ ] 1.3 A reload is unaffected: `cargo test -p tilrd --test reload --features tilrd/failpoints` green and itests UC-061, UC-062 green
- [ ] 1.4 USE_CASES.md entries for the new tests; README "Stopping" says programs ignoring SIGHUP in the foreground are killed 2 s after the stop and `nohup` jobs are left; verify by review against this proposal

## 2. itests

- [ ] 2.1 New `itests/scenarios/uc_086.py::a_shell_ignoring_sighup_does_not_outlive_the_server` (a shell with `trap '' HUP`, then `tilr kill-server`; no process of the pane's session but a `nohup` job remains), spec tag `daemon-lifecycle/no-shell-outlives-the-server#shell-ignoring-sighup`; itests/USE_CASES.md UC-086 points to it and loses "Сейчас баг — FAIL"; verify `./itests/dt uc_086` green
- [ ] 2.2 `daemon-lifecycle` and `panes` spec Notes: drop "No shell outlives the server" from the untested list, the UC-086 bug bullet and "the SIGKILL 2 s after `ClosePane`" from the untested parts; verify `openspec validate --specs --strict`

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 `./itests/dt uc_086 uc_017 uc_061 uc_062` green, then `./itests/dt coverage` exits 0 and `openspec/coverage/` is regenerated
- [ ] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
