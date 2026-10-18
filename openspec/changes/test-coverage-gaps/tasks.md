# Tasks

## 1. Cloud

- [ ] 1.1 Extend the test-played Cloud in `crates/tilrd/tests/cloud.rs` with `/v1/enroll`, `/v1/enroll/poll` (pending, approved, 404) and `DELETE /v1/machine`; verify `cloud::login_prints_the_code_and_polls_until_confirmed` and `cloud::login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing` (link file 0600, nothing written on 404)
- [ ] 1.2 Logout and revoke; verify `cloud::logout_unlinks_the_machine_whether_or_not_cloud_hears` and `cloud::a_link_cloud_revokes_stops_the_tunnel_until_linked_again` (401 and close 4001)
- [ ] 1.3 Control connection retry with the tunnel kept; verify `cloud::the_control_connection_is_retried_and_the_tunnel_kept_meanwhile` (Cloud goes away and comes back; `cloudflared` pid unchanged)
- [ ] 1.4 A stand-in `cloudflared` that exits; verify `cloud::a_cloudflared_that_exits_is_reported_and_started_again` (`tunnel_status` `failed`, restarted after 5 s)
- [ ] 1.5 No TCP port; verify `cloud::a_server_without_a_tcp_port_does_not_connect_to_cloud` (log line, Cloud sees no connection)
- [ ] 1.6 USE_CASES.md entries for 1.1-1.5; the `cloud-tunnels` Notes list shrinks; verify by review

## 2. Server

- [ ] 2.1 `exec::input_that_is_not_wanted_is_ignored`, `metrics::the_timing_fields_measure_what_they_say`, `reload::stale_state_files_are_swept_at_start` (files older than an hour via `touch -t`), each green; USE_CASES.md entries
- [ ] 2.2 `panes::a_pane_whose_command_cannot_start_is_reported_to_its_creator` and `panes::used_at_ms_follows_subscribe_and_input`; USE_CASES.md entries
- [ ] 2.3 `socks::the_socks_path_is_not_served_on_the_unix_socket` and `socks::a_session_that_does_not_say_where_it_goes_is_closed` (takes the 10 s); `agent::a_65th_stream_is_refused`; USE_CASES.md entries

## 3. Client

- [ ] 3.1 `client::reconnects_say_the_same_client_id` (the server's `PaneInfo.viewed_by` before and after a reload) and `client::kill_closes_the_named_pane_and_the_current_one`; USE_CASES.md entries
- [ ] 3.2 `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload` against a local CDN directory; USE_CASES.md entry

## 4. Installation

- [ ] 4.1 itests UC-007 (`uc_007.py::a_manual_install_from_the_cdn_layout_runs`, tag `installation/manual-install-from-the-cdn`; take the next free id if UC-007 is taken by then): read `latest`, fetch the archive, unpack to `~/.local/bin`, run `tilr`; itests/USE_CASES.md entry; verify `./itests/dt` UC-007 green

## 5. Flakes

- [ ] 5.1 Find and fix each flake or quarantine it with its cause in USE_CASES.md; verify 10 consecutive green runs of the affected tests (`cargo test --test files nothing_outside_the_shared_directories_is_served`, `cargo test --test reconnect a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`, `./itests/dt` UC-043)
- [ ] 5.2 Find what leaves `target/debug` test servers running; verify `pgrep -f target/debug/tilrd` is empty after `cargo test --workspace --features tilrd/failpoints --locked`
- [ ] 5.3 `reload::a_broken_executable_is_not_reloaded_into`: find what takes the watch-check-log path past 10 s under load (log timestamps of the replacement, the watch event, the check child) and fix or widen the wait with the cause recorded; verify 10 consecutive green full `cargo test --workspace --features tilrd/failpoints --locked` runs on macOS
- [ ] 5.4 (TILR-110) `local.rs` `spawn_env` spawns through `common::spawn_copied`; verify `scripts/linux.sh cargo test --features tilrd/failpoints --test local` green 20 times in a row and no `Text file busy` in the full Linux run
- [ ] 5.5 `reconnect::a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`: wait for the log line (poll `Server::logged` with a deadline) instead of asserting on a snapshot; verify 10 consecutive green full runs
- [ ] 5.6 (TILR-111) `reconnect::output_printed_while_nobody_watched_is_there_on_reconnect`: find why the output is missing after the reconnect under load (the test's `sleep 1` against the reconnect's timing, or a lost frame); fix with the cause recorded; verify 10 consecutive green Linux full runs
- [ ] 5.7 itests `uc_063`: find the typed line lost during an update (the client's resend, or input written to the old pane process after its handover) and fix it or file the bug as its own change; verify `./itests/dt --filter uc_063` green 10 times in a row
- [ ] 5.8 itests `uc_033`: give the case a port no other case or container takes (or wait for 7890 to be free) with the cause recorded; verify `./itests/dt --filter uc_033` green 10 times in a row inside a full scenarios run
- [ ] 5.9 (TILR-112) `reconnect::a_lagging_client_gets_the_last_screen_without_new_output`: find why the lagging client does not reach the last screen within 10 s under full-suite load (log the bulk window's marks and the frames sent to it); fix the server if the last full frame is not sent, else widen the wait with the cause recorded; verify 10 consecutive green full `cargo test --workspace --features tilrd/failpoints --locked` runs on macOS
- [ ] 5.10 (TILR-113) `files::two_servers_of_one_user_keep_their_host_sockets`: start the second server only after the first logs `host events on` (wait in the test or in `common::start_second`); verify `scripts/linux.sh cargo test --features tilrd/failpoints --test files` green 20 times in a row
- `local::a_server_that_stops_removes_its_socket_and_lock` is not a test bug: its fix is `startup-signal-window` (TILR-106), whose tasks include the run under load.

## 6. Checks and archive

- [ ] 6.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 6.2 Refresh each MODIFIED requirement in `specs/` from the current `openspec/specs/` if another change archived since touched it; drop "(planned …)" from the **Verified by** lines; remove the tested items from the specs' Notes; verify `openspec validate test-coverage-gaps --strict`
- [ ] 6.3 `./itests/dt` UC-007 and UC-043 green, then `./itests/dt coverage` exits 0 and its uncovered list holds only the `release-process` requirements and `daemon-lifecycle/no-shell-outlives-the-server` (unless their changes landed)
- [ ] 6.4 Record the implementing commit in proposal.md and archive the change; verify `openspec list` no longer shows it
