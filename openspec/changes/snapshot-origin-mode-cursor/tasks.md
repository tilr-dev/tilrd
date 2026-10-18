# Tasks

## 1. Snapshot

- [ ] 1.1 (TILR-116) Unit test `ghostty::tests::snapshot_keeps_the_cursor_under_origin_mode`: set a scrolling region (`ESC [ 3 ; 8 r`), turn on DECOM (`ESC [ ? 6 h`), move the cursor inside the region, snapshot into a fresh screen; verify the restored cursor (x, y) and the printed next character equal the original's, and that it fails before the fix
- [ ] 1.2 Fix the cursor restore for DECOM (and left/right margins with `?69`): translate by the margins when origin mode is on, or place the cursor before the mode is enabled; verify `cargo test -p tilr-screen` green, including `snapshot_keeps_a_pending_wrap`
- [ ] 1.3 Reload test `reload::the_cursor_under_origin_mode_is_where_it_was_after_a_reload` (a pane sets a region and DECOM, prints a marker at the cursor; after the reload the next character lands next to the marker); verify green 10 times in a row
- [ ] 1.4 USE_CASES.md entry for the test; verify by review

## 2. Checks and archive

- [ ] 2.1 Run the checks from AGENTS.md (fmt, clippy with and without `tilrd/failpoints`, the workspace tests, `cargo deny check`, gitleaks; on macOS also `scripts/linux.sh`); verify all green
- [ ] 2.2 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
