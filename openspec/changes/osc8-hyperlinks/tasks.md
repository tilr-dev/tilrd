# Tasks

## 1. Protocol and screen model

- [x] 1.1 `Frame.links` (`repeated string`) and `Run.link` (`uint32`) in `tilr.proto`, with comments (D1, D3); README "Protocol" describes them; verify `cargo test -p tilr-proto` and review of README
- [x] 1.2 Run building reads URIs only in rows with links and splits runs on link changes (D2, D3); verify with unit tests `ghostty::tests::a_hyperlink_goes_with_its_run` and `ghostty::tests::an_overlong_hyperlink_is_left_out`, and the existing `ghostty::tests::styles_are_split_into_runs` unchanged
- [x] 1.3 After a reload links are plain text, new ones work; verify with `reload::hyperlinks_are_plain_text_after_a_reload_and_new_ones_work`; README "Reloading without closing shells" states the limit; USE_CASES.md lists the tests

## 2. tilr attach

- [x] 2.1 `draw` wraps linked runs in OSC 8 (D5); verify with the unit test `attach::tests::a_linked_run_is_drawn_inside_osc_8`

## 3. Checks and archive

- [x] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 3.2 `./itests/dt --only scenarios --filter uc_079` and `./itests/dt --only scenarios --filter uc_06a` (input and reload with frames) green, then `./itests/dt coverage` exits 0
- [x] 3.3 Archive the change; verify `openspec validate --all --strict`
