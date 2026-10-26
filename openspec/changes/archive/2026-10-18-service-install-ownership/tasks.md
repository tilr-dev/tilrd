# Tasks

## 1. Service files it did not write

- [x] 1.1 Install checks an existing unit / plist for the marker before writing and stops with the error, running no `systemctl` / `launchctl`; verify with unit test `service::tests::a_file_without_the_mark_is_not_replaced` and itests `uc_020.py::install_leaves_a_unit_it_did_not_write` (spec tag `tilr-cli/service-install-leaves-a-unit-or-plist-it-did-not-write#a-unit-it-did-not-write`); verify `./itests/dt uc_020` green
- [x] 1.2 `crontab` with `LC_ALL=C`; missing crontab recognized from `no crontab for` and `No such file or directory`; verify with unit test `service::tests::a_missing_crontab_is_recognized_from_cron_and_busybox` and `./itests/dt uc_021` green
- [x] 1.3 Rename `write_private` to `write_whole` and fix its comment (0644 as spec'd); verify `cargo clippy -p tilr --all-targets --locked -- -D warnings`
- [x] 1.4 README "Starting at boot" says install refuses a unit or plist it did not write; USE_CASES.md (unit tests) and itests/USE_CASES.md (UC-020) list the new cases; verify by review against this proposal

## 2. Private state directory

- [x] 2.1 At start, an existing state directory of the user's is set to 0700 and one of another user refused, before `server-id`, the token or the Cloud link are touched (`identity.rs:28-38`); verify with `auth::an_existing_state_directory_is_made_private` (state directory pre-made 0755 with a token; after start 0700 and the same token accepted); USE_CASES.md entry

## 3. Checks and archive

- [x] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 3.2 `./itests/dt uc_005 uc_020 uc_021 uc_022 uc_023` green, then `./itests/dt coverage` exits 0 and `openspec/coverage/` is regenerated
- [x] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec list` no longer shows it
