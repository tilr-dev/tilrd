# Tasks

## 1. Check mode in the executable

- [ ] 1.1 `TILR_RELOAD_CHECK=<fd>`: parse argv, load the config, read the check state, run the resume path to the takeover point, answer `ok` / `error: <reason>` (design D2); `state-formats` prints `reload-check 1`; verify with the unit tests of the check mode and `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` expecting the new line
- [ ] 1.2 Failpoints `supervisor-resume-live` and `pane-resume-live` (design D6); verify `reload::a_version_that_crashes_taking_over_ends_the_server_and_its_shells` and `reload::a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on` pass with the live failpoints

## 2. Supervisor runs the check before its exec

- [ ] 2.1 Spawn the check child with argv, env and the check state copy, wait 10 s, exec only on `ok` (design D1, D3); verify with `reload::a_successor_that_panics_resuming_is_not_reloaded_into` (`supervisor-resume=panic`: pid, connection and shell kept, log line), `reload::a_successor_that_rejects_the_command_line_is_not_reloaded_into` and `reload::a_successor_that_refuses_the_configuration_is_not_reloaded_into`
- [ ] 2.2 Timeout and cleanup; verify with `reload::a_successor_that_never_answers_the_check_is_not_reloaded_into` (`supervisor-resume=sleep:20000`, no `*.check.state` left)
- [ ] 2.3 Older successors by the flag table (design D4); verify with an itests rollback (as UC-066) to the previous archive release started by `tilr` (all panes alive, or refusal logged naming the flag)

## 3. Pane processes run the check

- [ ] 3.1 Pane check before the pane exec (design D5); verify with `reload::a_pane_whose_successor_fails_the_check_carries_on` (`pane-resume=panic`, pane kept with its shell pid)

## 4. Documentation and tags

- [ ] 4.1 README "Reloading without closing shells" describes the check and the log line; USE_CASES.md lists the new tests; verify by review against this proposal
- [ ] 4.2 itests spec tags for the added requirements (UC-066 rollback case, UC-067 for `state-formats`) and `./itests/dt coverage` regenerated; verify it exits 0

## 5. Checks

- [ ] 5.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
