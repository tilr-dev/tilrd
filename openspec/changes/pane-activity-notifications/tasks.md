# Tasks

## 1. Message and bell

- [x] 1.1 `PaneActivity` and `PaneActivityKind` in `tilr.proto` (design D5), a `ServerMessage` variant; README "Protocol" lists it; verify `cargo test -p tilr-proto` and review of README
- [x] 1.2 `tilr-screen`: `on_bell` counts bells, `Screen::take_bells`; verify unit tests `ghostty::tests::a_bell_is_counted` and `ghostty::tests::a_bel_ending_an_osc_is_not_a_bell`
- [x] 1.3 Link kind `ACTIVITY` (D4), the pane's one-per-second bell report (D1) and the supervisor's fan-out to every greeted client (D3); verify with `panes::a_bell_in_an_unwatched_pane_is_reported`, `panes::bells_are_reported_once_a_second`, `panes::a_bel_ending_an_osc_is_not_reported`; USE_CASES.md entries

## 2. Command finished

- [x] 2.1 Foreground group watch (D2) after each read and on a 1 s tick; verify with `panes::a_finished_foreground_command_is_reported` and `panes::typing_at_the_prompt_reports_nothing`; USE_CASES.md entries
- [x] 2.2 Across a reload; verify with `reload::a_command_running_across_a_reload_is_reported_when_it_ends`; USE_CASES.md entry
- [x] 2.3 README section on pane activity (events, the bell limit, job control needed, no replay); verify by review against this proposal

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 No itests case is touched (cargo tests cover one machine); `./itests/dt coverage` exits 0 and lists the two new `panes` requirements as covered by cargo tests
- [ ] 3.3 Record the implementing commit in proposal.md and archive the change; verify `openspec list` no longer shows it
