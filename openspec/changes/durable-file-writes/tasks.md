# Tasks

## 1. Synced writes

- [ ] 1.1 A helper that syncs a written file, renames it and syncs the directory; FsWrite's `receive` uses it (`files.rs:611-615`); verify `files::a_file_is_written_whole_or_not_at_all` and the rest of `cargo test -p tilrd --test files` still green
- [ ] 1.2 itests UC-046: new case `uc_046.py::a_finished_write_is_synced_before_it_is_acknowledged` starting the server under `strace -f -e trace=fsync,fdatasync,rename,renameat,renameat2 -o <log>` (a tracer of its own child, so no extra ptrace capability; strace added to the image in `itests/src/tilr_itests/main.py`) and checking the order in the log; spec tag `files/writes-are-whole-or-not-at-all#synced-before-the-answer`; verify `./itests/dt uc_046` green
- [ ] 1.3 README "Files": a success answer to a write means the data and the name are on disk; USE_CASES.md and itests/USE_CASES.md (UC-046) list the new case; verify by review against this proposal

## 2. Synced update

- [ ] 2.1 `unpack` syncs each program; the directory is synced after the renames (`update.rs:157-161`, `199-210`); verify `cargo test -p tilrd --test update` green
- [ ] 2.2 itests UC-060: new case `uc_060.py::an_update_is_synced_before_it_reports_success` (strace on `tilrd update`), spec tag `update/the-release-is-put-in-place-by-two-renames-the-server-last#synced-before-success`; README "Updating" says the release is on disk before success; itests/USE_CASES.md (UC-060); verify `./itests/dt uc_060` green

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 `./itests/dt uc_046 uc_060 uc_065 uc_070` green, then `./itests/dt coverage` exits 0 and `openspec/coverage/` is regenerated
- [ ] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
