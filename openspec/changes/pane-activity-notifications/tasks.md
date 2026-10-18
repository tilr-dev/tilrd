# Tasks

## 1. Message and bell

- [ ] 1.1 `PaneActivity` in `tilr.proto` (`ServerMessage` variant, `kind` enum `BELL`, `COMMAND_FINISHED`); README "Protocol" lists it; verify `cargo test -p tilr-proto` and review of README
- [ ] 1.2 Bell detection with the 1 s limit (design D1) and fan-out (D3); verify with `panes::a_bell_in_an_unwatched_pane_is_reported`, `panes::bells_are_reported_once_a_second`, `panes::a_bel_ending_an_osc_is_not_a_bell`; USE_CASES.md entries

## 2. Command finished

- [ ] 2.1 Foreground group watch (design D2); verify with `panes::a_finished_foreground_command_is_reported` and `panes::typing_at_the_prompt_reports_nothing`; USE_CASES.md entries
- [ ] 2.2 Across a reload; verify with `reload::a_command_running_across_a_reload_is_reported_when_it_ends`
- [ ] 2.3 README section on pane activity (events, limit, no replay); verify by review against this proposal

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
