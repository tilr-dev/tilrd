# Tasks

## 1. Snapshot fields

- [ ] 1.1 Soft wrap and pending wrap; verify with `reload::a_wrapped_line_at_the_history_border_stays_one_line` and a pending-wrap test
- [ ] 1.2 kill_at timer; verify with `reload::a_closing_pane_is_still_killed_after_a_reload`
- [ ] 1.3 Placements partly in history; verify with a kitty-graphics reload test
- [ ] 1.4 Bump the pane state format; verify `tilrd state-formats` output and a reload from the previous release

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
