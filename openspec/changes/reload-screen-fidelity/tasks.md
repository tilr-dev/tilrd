# Tasks

## 1. Snapshot

- [ ] 1.1 Border wrap (design D1); verify with unit test `ghostty::tests::snapshot_keeps_a_line_wrapped_across_the_history_border` and `scroll::a_line_wrapped_across_the_history_border_stays_one_line_after_a_reload`
- [ ] 1.2 Pending wrap (design D2); verify with unit test `ghostty::tests::snapshot_keeps_a_pending_wrap`; if libghostty-vt has no accessor, record it in `libghostty-upstream-fixes` and drop the "Pending wrap" scenario from this change's delta before archiving
- [ ] 1.3 Images in the history (design D4); replace `snapshot_leaves_out_an_image_scrolled_into_the_history` with `ghostty::tests::snapshot_keeps_an_image_scrolled_into_the_history`; verify `cargo test -p tilr-screen` green
- [ ] 1.4 USE_CASES.md entries; README "Reloading without closing shells" no longer lists these losses; verify by review against this proposal

## 2. Close deadline

- [ ] 2.1 `kill_in_ms` and `kill_pgids` in `PaneHandover` (comments say "0 = none"); written before the exec, armed by the successor (design D3); verify with `reload::a_closing_pane_is_still_killed_after_a_reload` and `pane::tests::state_in_the_current_format_is_read` still green (format unchanged, design D5)
- [ ] 2.2 `tilrd state-formats` output unchanged; verify `reload::` tests of the formats and a reload from a build of the previous commit (`cargo test -p tilrd --test reload --features tilrd/failpoints`)

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 `./itests/dt uc_061 uc_062 uc_063` green (no itests case added), then `./itests/dt coverage` exits 0 and `openspec/coverage/` is regenerated
- [ ] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
