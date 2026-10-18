# Tasks

## 1. Check mode in the executable

- [x] 1.1 `TILR_RELOAD_CHECK=<path>`: parse argv, load the config, read the check state, run the resume path to the takeover point, answer `ok` / `error: <reason>` on stdout (design D2); `state-formats` prints `reload-check 1`; verify with the unit tests in `reload.rs` (`a_check_passes_only_on_ok_and_a_clean_exit`, `a_check_child_that_dies_is_reported_with_what_it_said_last`, `an_executable_that_cannot_run_the_check_cannot_take_over`) and `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` expecting the new line
- [x] 1.2 Failpoints `supervisor-resume-live` and `pane-resume-live` (design D6); verify `reload::a_version_that_crashes_taking_over_ends_the_server_and_its_shells` and `reload::a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on` pass with the live failpoints

## 2. Supervisor runs the check before its exec

- [x] 2.1 Spawn the check child with argv, env and the check state copy before stopping anything, serve events while waiting up to 10 s, exec only on `ok` (design D1, D3); verify with `reload::a_successor_that_panics_resuming_is_not_reloaded_into` (`supervisor-resume=panic`: pid, connection and shell kept, log line), `reload::a_successor_that_rejects_the_command_line_is_not_reloaded_into`, `reload::a_successor_that_refuses_the_configuration_is_not_reloaded_into` and `reload::a_successor_is_checked_while_the_server_serves_and_then_reloaded_into`
- [x] 2.2 Timeout and cleanup; verify with `reload::a_successor_that_never_answers_the_check_is_not_reloaded_into` (`supervisor-resume=sleep:20000`, no `*.check.state` left)
- [x] 2.3 Successors without `reload-check 1` are refused (design D4, the flag table dropped); verify with the third case of `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`

## 3. Pane processes run the check

- [x] 3.1 Pane check before the pane exec (design D5); verify with `reload::a_pane_whose_successor_fails_the_check_carries_on` (`pane-resume=panic`, pane kept with its shell)

## 4. Documentation and tags

- [x] 4.1 README "Reloading without closing shells" describes the check and the log line; USE_CASES.md lists the new tests; verify by review against this proposal
- [x] 4.2 itests UC-06A (`uc_06a.py::a_version_that_crashes_resuming_is_not_reloaded_into_and_the_shells_live_on`, a release whose tilrd crashes resuming), tagged with the added requirement, in itests/USE_CASES.md; `./itests/dt coverage` regenerated; verify it exits 0 and `./itests/dt --only scenarios --filter uc_06` passes

## 5. Checks

- [x] 5.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
