# Tasks

## 1. Outside the shared directories

- [ ] 1.1 Supervisor checks kind `file` against the roots before delivering, when files are on (D4); verify with `files::tilr_edit_outside_the_shared_directories_is_refused_at_once`; README "Files and `tilr edit`"; USE_CASES.md entry

## 2. Protocol and waiting

- [ ] 2.1 `HostEvent.event_id`, `HostEventDone`, the `"host-events-wait"` client feature in `tilr.proto` comments; README "Protocol"; verify `cargo test -p tilr-proto` and review of README
- [ ] 2.2 Host socket `"wait"` requests, the `event_id` in the reply, the `done` line, the wait registry and refusal without a waiting client (D1, D2); verify with `files::tilr_edit_wait_returns_when_the_client_is_done` and `files::tilr_edit_wait_needs_a_client_that_can_wait`; unit test `host::tests::a_wait_reply_carries_the_event_id`
- [ ] 2.3 `--wait` for `tilr edit` / `tilr open` in the server's shim and in `crates/tilr`; verify with `client::tilr_edit_wait_waits_for_the_done_line` (crates/tilr/tests/client.rs)
- [ ] 2.4 Re-registering after a lost connection and holding early dones for 30 s (D3); verify with `reload::tilr_edit_wait_goes_on_across_a_reload`
- [ ] 2.5 README "Files and `tilr edit`" documents `--wait` (`GIT_EDITOR="tilr edit --wait"`); USE_CASES.md entries; verify by review

## 3. itests

- [ ] 3.1 UC-076 gains `uc_076.py::git_commit_with_tilr_edit_wait_as_the_editor_takes_the_message_from_the_client` (driver lists `"host-events-wait"`, writes the file with `FsWrite`, sends `HostEventDone`), tagged `clipboard-host-events/tilr-edit-wait-returns-when-the-file-is-closed#as-the-git-editor`; itests/USE_CASES.md UC-076 line updated; verify `./itests/dt UC-076` green

## 4. Checks and archive

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 4.2 `./itests/dt UC-035 UC-076` green, then `./itests/dt coverage` exits 0
- [ ] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
