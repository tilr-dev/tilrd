# Use cases and tests

Each use case is a test that starts a real server, connects to it over a
WebSocket as a client, and checks what the shell in a pane sees. The tests live
in `crates/tilrd/tests/`, the shared harness in `tests/common/mod.rs`; the tests
of the client `tilr` live in `crates/tilr/tests/` (they run `tilr`, which starts
the `tilrd` built next to it).

Run: `cargo nextest run --workspace --features tilrd/failpoints --locked`; on Linux from
another OS, `scripts/linux.sh`. Use cases with a failure halfway through
(marked †) make something go wrong at a given place in the code through
failpoints (`src/fail.rs`) and are compiled only with that feature.

The test's name is the statement of the use case.

## Linux build scripts — `tests/linux_script.rs`

| Use case | Test |
|---|---|
| Two worktrees with the same basename overlap builds without replacing each other's binaries; targets are reused through symlinks, separated by platform, and used by release copying; commands use their own built image and a failed build starts no command | `parallel_worktrees_keep_their_linux_build_outputs` |

The script integration fixture uses a Docker stand-in in the cargo suite. Run
`python3 scripts/test-linux.py --docker` to compile the two fixture binaries in
real Linux containers with the same FIFO barrier.

## Test harness — `tests/harness.rs`

| Use case | Test |
|---|---|
| A failed async or blocking log wait, or a panic with a poisoned log, preserves diagnostics and cleans up the server without aborting the test process | `a_failed_log_wait_unwinds_and_cleans_up_the_server` |

## Connection and panes — `tests/panes.rs`

| Use case | Test |
|---|---|
| A bell reaches greeted clients that do not watch the pane | `a_bell_in_an_unwatched_pane_is_reported` |
| Bells within a second are dropped, and the next second permits a bell | `bells_are_reported_once_a_second` (`failpoints`) |
| BEL ending an OSC title sequence is not reported as activity | `a_bel_ending_an_osc_is_not_reported` |
| A foreground job returning control to its shell reports command completion | `a_finished_foreground_command_is_reported` |
| Typing at the prompt reports no activity | `typing_at_the_prompt_reports_nothing` |
| The client says hello, the server reports its protocol version (1), the features it can be run without (`clipboard`, `files`, `host-events`, `agent`, `exec`) and its clipboard limit | `hello_announces_the_clipboard_and_its_limit` |
| A client with another protocol version (newer: 2, none: 0) gets an error naming both versions ("this server speaks 1"), and the connection is closed | `a_client_built_for_another_protocol_version_is_turned_away` |
| The first thing a client gets is the answer to its `Hello`: pane lists are not sent to it before, and after that it gets them like everyone else | `the_first_thing_a_client_hears_is_the_answer_to_its_hello` |
| A client that does not start with `Hello` gets an error and is disconnected; its input never reaches the shell | `a_client_that_skips_hello_is_turned_away` |
| A browser page connects only from an allowed origin; programs without `Origin` get through | `a_browser_page_connects_only_from_an_allowed_origin` |
| The configuration file (`server.toml`) names the allowed origins and `--allow-origin` replaces its list; a file that does not parse, or names an unknown key, stops the server from starting and says where it is | `the_configuration_file_names_the_allowed_origins` |
| A pane without a command runs the user's login shell (`$0` is `-zsh`, `-bash`, `-sh`), with `SHELL`, `HOME`, `USER`, `LOGNAME` set and the server's tools (`$TILR_SHIMS`) still reachable after the login profile | `a_pane_without_a_command_runs_the_users_login_shell` |
| A command in a pane runs and its output is visible; the shell's exit code reaches the client | `a_pane_runs_commands_and_reports_how_its_shell_exited` |
| A shared test clock gives exact creation time, then subscription and input each update `used_at_ms` to the test's next value; a frame built at a fixed clock value still has positive `build_us` | `used_at_ms_follows_subscribe_and_input` (`failpoints`) |
| A pane opens in the home directory if the client named no directory or one that does not exist on the server; in the named one if it exists | `a_pane_starts_in_the_home_directory_unless_given_one_that_exists` |
| Closing a pane ends its shell | `closing_a_pane_ends_its_shell` |
| Closing a pane whose shell runs a foreground job that ignores SIGHUP: `PaneExited` within 3 s, the job killed | `closing_a_pane_ends_a_foreground_program_ignoring_sighup` |
| A pane's program that ignores SIGHUP (and reads no input): gone once the stopped server's pane process has exited | `a_shell_ignoring_sighup_is_ended_when_the_server_stops` |
| A foreground job of the pane's shell that ignores SIGHUP: gone once the stopped server's pane process has exited | `a_foreground_program_ignoring_sighup_is_ended_when_the_server_stops` |
| A `nohup … &` job in the background outlives the server, as an SSH hangup leaves it | `a_nohup_background_job_is_left_when_the_server_stops` |
| Two clients watch one pane and both type into it | `two_clients_watch_and_type_into_the_same_pane` |
| A client left: the pane lives on, the others get an updated list | `a_client_that_leaves_does_not_take_the_pane_with_it` |
| A client unsubscribed: it gets no more frames | `a_client_that_stops_watching_gets_no_more_frames` |
| A resize reaches the terminal the shell sees | `resizing_a_pane_resizes_the_terminal_the_shell_sees` |
| An unknown message type is ignored | `a_message_type_the_server_does_not_know_is_ignored` |
| A malformed packet gets an error, the connection stays | `a_malformed_packet_gets_an_error_and_the_connection_stays` |
| Asking for a pane that does not exist gets an error; so does a name that is not a UUID | `asking_for_a_pane_that_does_not_exist_gets_an_error` |
| A pane has its own UUID: the shell sees it in `$TILR_PANE`, the pane process on its command line (`pane --id <uuid>`), where the size is given only if it is not 80×24 | `a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` |
| A pane's process title follows its shell pid, OSC 7 directory and foreground command, then returns to the shell after Ctrl+C | `the_pane_process_title_follows_the_directory_and_the_command` (`failpoints`) |
| A silent exec in the foreground group updates the process title without changing the leader pid | `the_pane_process_title_follows_exec_in_the_same_foreground_group` (`failpoints`) |
| Reload preserves start arguments and restores the pane process title with the same shell and directory | `the_pane_process_title_is_set_again_after_a_reload` (`failpoints`) |
| Titles are cut to the available area and moving the environment preserves its values | `title::tests::a_title_is_cut_to_the_area`, `title::tests::the_environment_is_unchanged_after_the_move`, `title::tests::a_moved_environment_entry_keeps_the_contiguous_prefix` |
| A foreground leader whose command line cannot be read is shown by its process name, or `?` once it is gone | `title::tests::a_leader_that_is_gone_is_shown_as_a_placeholder`, `title::tests::a_zombie_leader_is_named_by_its_process` (Linux), `title::tests::another_users_leader_is_named_by_its_process` (macOS) |
| `tilrd ps` (for users, `tilr ps`) shows the panes of a running server: id, pid, size, command | `ps_lists_the_panes_of_a_running_server` |
| The Tilr app's health check `tilrd ps --url <url> --server-version` against a stopped server: exits non-zero within 5 s and prints no version | `ps_server_version_fails_when_nothing_listens` |
| A server has its own id, the same after a restart; another server has another one | `a_server_keeps_its_id_when_it_restarts` |
| A server started by a service manager (`NOTIFY_SOCKET`, `LISTEN_*`, `WATCHDOG_*` set) still says `READY=1`, but its panes do not get those variables | `a_pane_does_not_get_the_service_managers_variables` |
| A test holds the outer login shell in its SIGHUP trap after terminal hang-up, checks the server while held, releases it and awaits its exit before reconnecting | `a_test_holds_the_login_shell_after_hangup_and_releases_its_exit` (`failpoints`) |

## Unix socket for local clients — `tests/local.rs`

| Use case | Test |
|---|---|
| The server tells systemd `READY=1` on `$NOTIFY_SOCKET` once it listens (`Type=notify`) | `a_service_manager_is_told_when_the_server_is_ready` |
| The server listens on a Unix socket (directory 0700, socket and lock 0600, the pid in the lock; a `Hello` without a token is accepted, `tilrd ps` goes through the socket) and by default on TCP at 127.0.0.1 | `the_socket_is_the_users_alone_and_asks_no_token` |
| `--no-listen`: the socket only; `--no-listen --no-socket`: the error "nothing to listen on" | `no_listen_serves_the_socket_alone` |
| A TCP port that is taken: the server does not start, the error says the port is taken and suggests `--listen`/`--no-listen`; no socket is left behind | `a_tcp_port_that_is_taken_stops_the_server_with_a_clear_error` |
| Another uid cannot connect: first the permissions keep it out, and permissions opened by hand do not help, since the server checks the peer's uid (as root only) | `another_user_is_turned_away_even_where_the_permissions_let_it_in` |
| A socket left by a killed (SIGKILL) server is removed by the next server, which listens in its place | `a_socket_left_by_a_killed_server_is_replaced` |
| A plain file where the socket goes: a clear error, the file intact | `a_file_at_the_path_is_left_alone_with_a_clear_error` |
| Another program listening where the socket goes: a clear error, its socket intact and working | `another_programs_listener_at_the_path_is_left_alone_with_a_clear_error` |
| The lock file is a symbolic link: the server does not start, the link is not followed and its target is intact | `a_symlinked_lock_file_is_refused` |
| A path longer than `sun_path`: a clear error and an exit (`TILR_SOCKET`) | `a_socket_path_too_long_is_a_clear_error` |
| Eight servers started at once on one socket: exactly one is left, the others say "another tilrd" | `of_servers_started_at_once_one_serves_the_socket` |
| With TCP, a socket in use is only a warning, the server works over TCP (as the Tilr app's does); a socket named with `--socket` must work | `with_a_tcp_port_a_socket_in_use_is_only_a_warning` |
| A reload keeps the socket and the lock (the same pid, the panes in place, a second server does not get the socket) | `a_reload_keeps_the_socket_and_its_lock` |
| Stopping removes the socket and the lock | `a_server_that_stops_removes_its_socket_and_lock` |
| `RUST_LOG` is the log filter: with `debug` a connection to the socket that is not a WebSocket logs a DEBUG `websocket handshake failed`; unset or empty, `info` and no such line; an invalid value, `info` and one `RUST_LOG ignored:` | `rust_log_sets_the_log_level` |
| † SIGTERM between the socket being served and the signal handlers being installed (failpoint `supervisor-before-signals`): a clean stop (exit 0, `shutting down`), socket and lock removed | `a_sigterm_while_the_server_starts_stops_it_cleanly` |
| † The same with SIGINT (Ctrl+C on a foreground `tilrd`) | `a_sigint_while_the_server_starts_stops_it_cleanly` |
| † SIGHUP in that window: ignored, the server serves a client afterwards | `a_sighup_while_the_server_starts_is_ignored` |
| Server commands (`tilr token`) go over the Unix socket only, even with `--no-exec`; over TCP they are refused | `server_commands_run_for_local_clients_only` |
| A server on its socket alone (`--no-listen`) does not reload into an executable that cannot take the socket (`state-formats` without `unix-socket 1`) | `a_server_on_its_socket_alone_does_not_reload_into_one_that_cannot_take_it` |
| Nor does a server with a TCP port and its socket: the socket would be left with nobody serving it | `a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket` |
| † A reload refused by the check of the successor: the same connection, new connections through the socket, the lock still held | `a_refused_reload_keeps_the_socket_and_its_lock` |
| Private test installations share warmed inodes; repeated replacements alternate them and consume the staged path | `warm_executable_links_reuse_inodes_and_replace_repeatedly` |

## Acknowledged input — `tests/input_delivery.rs`

| Use case | Test |
| --- | --- |
| A numbered Resize publishes its applied dimensions while earlier PTY input is blocked; cumulative delivery stays pending until the program reads | `input_delivery::a_numbered_resize_updates_the_pane_list_while_pty_input_waits` |
| Numbered input before Hello receives the handshake error and closes the connection | `input_delivery::numbered_input_before_hello_is_refused_and_closed` |
| Numbered input without a stable client id returns an explicit error | `input_delivery::numbered_input_requires_a_stable_client_id` |
| All six numbered input variants naming an absent UUID or malformed pane name receive an error and terminal acknowledgement; lost acknowledgements are repeated after reconnect, reclaiming entries without discarding another pane's pending input, while unnumbered input and controls receive only errors | `input_delivery::missing_pane_input_is_rejected_and_reclaimed_across_reconnect` |
| A pane held before its Hello probe is killed; concurrent handshakes retain healthy counters and input applies once | `input_delivery::a_stuck_pane_does_not_block_other_panes_hello` |
| Completed identities finalize and lose their counters, refusing further numbered input on the old connection | `finalized_sessions_do_not_accumulate_delivery_history` |
| Expired disconnected sessions lose counters; active sessions retain them | `expired_sessions_are_removed_but_active_sessions_are_retained` |
| Internal cleanup fields cannot be forged at client ingress | `clients_cannot_forge_internal_delivery_cleanup` |
| Exec resets delivery history but preserves queued PTY bytes | `exec_resets_delivery_history_and_preserves_pending_bytes` |
| The driver drains pending input before normal disconnect finalizes its session | `session::input_delivery_tests::normal_disconnect_drains_input_before_finalizing` |
| Expiry removes delivery markers without discarding pending PTY bytes | `expiry_preserves_pending_bytes_without_stale_ack_markers` |

## The client `tilr` — `crates/tilr/tests/client.rs`

| Use case | Test |
|---|---|
| Normal detach finalizes acknowledged input | `normal_detach_finalizes_its_acknowledged_input` |
| Detach with pending input while disconnected exits promptly without finalizing | `detach_with_unacknowledged_input_while_disconnected` |
| Detach while a connected server withholds input ACKs exits promptly without finalizing | `detach_with_unacknowledged_input_while_connected` |
| The first `tilr` starts a detached server (a session of its own, not its child, with TCP on loopback), the next one connects to the same; `ls`, `ps`, `version` | `the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it` |
| The TCP port is taken (the Tilr app, another user): a server started by `tilr` starts anyway, with a warning in the log, on the socket only | `a_server_tilr_starts_goes_on_without_tcp_when_the_port_is_taken` |
| Six `tilr` at once on an empty machine: one server, a second is not even started | `tilr_started_many_times_at_once_starts_one_server` |
| `tilr kill-server`: the server and the pane processes end, the socket and the lock are removed | `kill_server_ends_the_shells_and_removes_the_socket` |
| A socket left after a SIGKILL: `ls` starts nothing, `new` starts a new server | `a_socket_left_by_a_killed_server_is_replaced_by_the_next_tilr` |
| A file where the socket goes: a clear error, the file intact | `a_file_in_the_way_is_left_alone_with_a_clear_error` |
| A socket path that is too long: a clear error | `a_socket_path_too_long_is_a_clear_error` |
| A runtime directory open to group or others (0755, 0770) or a symbolic link to a private one: `new`, `ls`, `kill-server` refuse naming it and what is wrong, no server is started, nothing is made there; private again, it works (another user's directory: itests UC-03A) | `a_runtime_directory_open_to_others_or_a_link_is_refused_and_nothing_started` |
| The lock file or `tilrd.log` is a symbolic link: `tilr new` refuses, the link is not followed and its target is intact | `a_symlinked_lock_file_is_refused` |
| `tilr token`, `tilr cloud status` run in the server; through `--connect` they are refused | `server_commands_run_in_the_server` |
| Deploy validates a release, installs both programs over SSH and prints connection commands and the remote token; unsupported platforms and damaged archives change nothing | `deploy_through_a_stub_ssh_installs_both_and_prints_how_to_connect`, `deploy_names_a_platform_without_a_release`, `deploy_refuses_a_damaged_archive_before_sending_it` |
| Deploy obtains the persistent token before replacing an installed client whose protocol differs from the new release; a failed service install restores its previous configuration | `deploy_gets_the_token_before_replacing_an_old_protocol_client`, `deploy_restores_service_configuration_when_the_manager_refuses_it`, `deploy_restores_the_crontab_after_a_service_failure`, `deploy_does_not_start_a_daemon_before_a_failing_service_install` |
| Fetch-release checks a foreign-platform archive and rejects a bad checksum without executing its programs | `fetch_release_writes_a_checked_archive`, `fetch_release_refuses_a_bad_checksum` |
| Copying a private installation keeps destination writers out of unrelated forked children (FIFO-controlled Linux regression) | `copying_a_program_does_not_leak_its_writer_into_an_unrelated_fork` |
| `tilr update --cdn` runs in the local server against a loopback release feed, updates an isolated installation, and waits for a connection after reload | `tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload` |
| The client requests no update execution timeout and keeps waiting through a day of virtual time until the command exits successfully | `conn::update_wait_tests::update_waits_for_command_exit_after_a_long_launch_delay` |
| `tilr edit` in a pane sends the request to `$TILR_SOCK` with an absolute path | `tilr_edit_in_a_pane_asks_the_server_of_the_pane` |
| In a terminal (a pty): input gets through, after a server reload the client comes back on its own, Ctrl+] d detaches, the pane stays | `attached_in_a_terminal_it_types_reconnects_after_a_reload_and_detaches` |
| On initial attach and reconnect, a terminal sends its current size before Subscribe, restoring a pane resized by another client | `reconnect_restores_terminal_size_before_subscribing` |
| An attached `tilr` uses the same client id in `PaneInfo.viewed_by` before and after a server reload | `reconnects_say_the_same_client_id` |
| `tilr kill` closes a pane by id or via `$TILR_PANE`, waits for each to disappear, and reports `no panes` when none remain | `kill_closes_the_named_pane_and_the_current_one` |
| A second connection says `Hello` with an attached `tilr`'s client id (read from `PaneInfo.viewed_by`) on the same socket: `tilr` is closed as superseded (4000), prints `[superseded by another connection of this client]`, restores the terminal and exits 0 without reconnecting; the pane stays | `an_attach_superseded_by_its_own_client_id_ends` |
| `kill -TERM` of an attached `tilr`: it ends by SIGTERM, the terminal back in cooked mode (`icanon`, `echo`), cursor shown, main screen, nothing printed; the pane stays | `a_terminated_attach_restores_the_terminal` |
| A reconnect that connects but loses the connection before the pane list comes (a stand-in server on a TCP port): a failed attempt on the status line, the next one attaches and typing gets through | `an_error_right_after_a_reconnect_is_retried` |
| Ctrl+] d while a reconnect attempt waits on a server that accepts and never answers: detached at once, not after the 5 s connect timeout | `detach_works_while_a_reconnect_attempt_hangs` |
| Unit: a text copy made in the attached pane goes to the terminal as `ESC ] 52 ; c ; <base64> ESC \`, once its last chunk came; a copy cut short by a newer one, an image, and one with more data than announced are not (`src/attach.rs`) | `attach::tests::copied_text_goes_to_the_terminal_as_osc_52` |
| `tilr cloud login --help` names the same default Cloud as tilrd, `https://wss.tilr.dev` | `cloud_login_help_names_the_default_of_tilrd` |

## Access — `tests/auth.rs`

| Use case | Test |
|---|---|
| A client without the access token, or with someone else's, gets an error and is disconnected; with the token it gets in | `a_client_needs_the_access_token` |
| The token is kept in a file readable by the user only (the state directory is 0700) and survives a restart; `tilrd token` prints the same one | `the_token_is_kept_for_the_user_only_and_across_restarts` |
| A state directory left 0755 with an id and a token in it is set to 0700 at start; the server keeps that id and token | `an_existing_state_directory_is_made_private` |
| A server with `--no-auth` asks for no token and warns about it in the log | `a_server_started_without_authentication_asks_for_no_token` |
| A connection that opens its WebSocket and never says `Hello` is closed with 4002 `timeout` about 10 s later, even with `--client-timeout 0`, and the log says so; one that says `Hello` after a second stays; only `tilrd_client_drops_total{reason="no_hello"}` goes up | `a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello` |

## Outgoing WebSocket TLS — shared by Cloud control, tilrd ps, tilr --connect and tilr-driver

Unit tests in `crates/tilr-websocket/src/lib.rs`; all clients use the same explicit connector, including when built separately.

| Use case | Test |
|---|---|
| Only patched bundled roots and certificate verifier releases remain in the lock file | `websocket_dependencies_use_only_patched_bundled_roots` |
| The trust store contains exactly the bundled Mozilla roots | `the_trust_store_is_exactly_the_bundled_mozilla_roots` |
| Plain WebSockets retain request headers | `plain_websockets_keep_request_headers` |
| An explicit trusted root permits a TLS WebSocket handshake | `an_explicit_root_allows_a_verified_secure_websocket` |
| Bundled roots reject an untrusted issuer | `bundled_roots_reject_an_untrusted_issuer` |
| A trusted certificate for another hostname is rejected | `an_explicit_root_does_not_allow_a_wrong_hostname` |

## tilr Cloud — `tests/cloud.rs`

In these tests the test itself plays Cloud, and `cloudflared` is a script that
records how it was started. Cloud itself is checked by its own tests, in the
Cloud repository.

| Use case | Test |
|---|---|
| Login sends an enrollment request, prints the verification link and code, and polls through pending until approved | `login_prints_the_code_and_polls_until_confirmed` |
| An approved login saves a private link file; an expired code leaves no link | `login_keeps_the_link_once_confirmed_and_an_expired_code_links_nothing` |
| Logout without a link reports it; with a link it calls Cloud and removes local state even when Cloud rejects the request | `logout_unlinks_the_machine_whether_or_not_cloud_hears` |
| Cloud revokes a link by close 4001 or HTTP 401; the tunnel stops and a new link reconnects | `a_link_cloud_revokes_stops_the_tunnel_until_linked_again` |
| When the control connection drops, the server reconnects while the same `cloudflared` keeps running | `the_control_connection_is_retried_and_the_tunnel_kept_meanwhile` |
| A failed `cloudflared` reports failure and is started again | `a_cloudflared_that_exits_is_reported_and_started_again` |
| A linked server without a TCP port logs that Cloud is disabled and makes no Cloud connection | `a_server_without_a_tcp_port_does_not_connect_to_cloud` |
| Cloud login help names `https://wss.tilr.dev` as the default API host | `cloud_login_help_names_the_default_api_host` |
| A linked server keeps its saved Cloud URL and connects to Cloud with its own token; given a tunnel, it starts `cloudflared` pointed at its own port only; tunnel revoked: `cloudflared` stopped, secret removed | `a_linked_server_gets_its_tunnel_from_cloud` |
| A server with `--no-auth` refuses the tunnel | `a_server_without_authentication_takes_no_tunnel` |
| A tunnel description that would extend the `cloudflared` config (another service) is rejected | `a_tunnel_that_would_lead_elsewhere_is_refused` |
| On a server reload the old `cloudflared` is stopped and the successor starts its own | `a_reload_replaces_cloudflared_with_one_of_the_new_server` |
| A server linked while running connects on its own; one unlinked disconnects | `a_server_linked_while_it_runs_connects_on_its_own` |
| `cloudflared` runs in a process group of its own, not the server's | `cloudflared_is_not_in_the_servers_process_group` |
| A server killed with SIGKILL leaves a `cloudflared` (one that ignores SIGTERM, so even Linux's parent-death signal leaves it): the next server on the runtime directory stops it before it starts its own, and says so | `a_cloudflared_left_by_a_killed_server_is_stopped` |
| A `cloudflared` that ignores SIGTERM is killed 3 s into a reload, which then completes; only the successor's runs | `a_cloudflared_ignoring_sigterm_does_not_hold_back_a_reload` |
| † Starting `cloudflared` panics (failpoint `spawn-unblocked`): the next reload is not held up by it and goes through; the successor starts its own | `a_panic_starting_cloudflared_does_not_hold_up_the_next_reload` |
| Unit: only a process running this runtime directory's `config.yml` is taken for a leftover `cloudflared` (`src/cloud.rs`) | `cloud::tests::only_a_process_running_this_configuration_is_taken_for_a_leftover` |

## Reconnecting — `tests/reconnect.rs`

| Use case | Test |
|---|---|
| A client came back: the pane is there, the first frame is the whole screen, the shell is the same | `a_client_that_reconnects_finds_its_pane_as_it_left_it` |
| What was printed while nobody watched is visible after coming back | `output_printed_while_nobody_watched_is_there_on_reconnect` |
| Input echoed before the shell starts does not attach the expected output to its initial prompt | `reconnect_output_when_input_precedes_the_shell_prompt` |
| Explicit pauses order unwatched output, accepted reconnect, subscription and full-frame enqueue | `a_test_controls_output_accept_subscribe_and_full_frame_on_reconnect` (`failpoints`) |
| A reconnected client's reads stay paused while it can send; continuous output reaches the lag decision, then explicit resume receives full-frame recovery | `a_test_pauses_socket_reads_until_reconnected_client_lags` (`failpoints`) |
| Subscribing again sends the whole screen again | `subscribing_again_sends_the_whole_screen_again` |
| Dropping a read-pause handle without resume fails the next read | `a_dropped_read_pause_fails_the_next_read` |
| Many reconnects leave no "ghost" viewers | `many_reconnects_leave_no_viewers_behind` |
| A client vanished silently (connection open, no answer to pings): the server drops it on its own, the others get an updated list; once back, the client reconnects to its pane | `a_client_that_vanishes_without_closing_is_dropped` |
| A client still sends (its own `Ping`s) but no longer hears the server: it does not read and does not answer the server's pings (the direction to the client is cut); the server drops it as unresponsive, not as silent | `a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped` |
| A client is silent but still there (answers pings): the connection stays | `an_idle_client_that_is_still_there_stays_connected` |
| With `--client-timeout 0` silent clients are not dropped | `a_silent_client_is_kept_when_the_timeout_is_off` |
| The server vanished silently: the client sends `Ping`, no `Pong` comes back, and the client sees it; the server comes back to life and the answer arrives | `a_client_can_tell_that_the_server_vanished_without_closing` |
| The same client (`Hello.client_id`, the same token) comes on a new connection while its old one is still open: the old one is closed at once, with code 4000 `superseded`, before the new one's `HelloAck`, without waiting for a timeout; two tabs (different `client_id`s) both live; `PaneInfo.viewed_by` holds the `client_id` and address of each viewer | `a_client_back_on_a_new_connection_supersedes_its_old_one_at_once` |
| The same `client_id` with another token: on a server with a token, refused, the old connection alive; on a server without one (`--no-auth`) another token is another user, nobody is superseded | `the_same_client_id_under_another_token_supersedes_nobody` |
| A client lags under heavy output (a small receive buffer, not reading) and the pane then goes quiet: once it reads again it gets a full frame and ends up with the pane's last screen, with nothing more printed | `a_lagging_client_gets_the_last_screen_without_new_output` |
| Unit: a delta already on its way when a client subscribes is not sent to it: its first frame is full and carries its own input echo, a second Subscribe meanwhile keeps that echo, other viewers keep getting frames; the wait is not counted as skipped frames and does not ask the pane for a second full frame (`src/supervisor.rs`) | `supervisor::tests::a_subscriber_gets_full_before_delta_and_keeps_its_own_input_echo` |
| A server started from an SSH session (`tilrd &`) whose connection dropped: the terminal is closed and the process group got SIGHUP; the server, the pane processes and the shells live on, the client connects to the same pane; stopping the server still ends the shells | `a_server_started_from_a_terminal_outlives_the_terminal_hanging_up` |

## Scrolling — `tests/scroll.rs`

| Use case | Test |
|---|---|
| Scrolling back shows history, scrolling forward returns | `scrolling_back_shows_history_and_scrolling_forward_returns` |
| "To bottom" returns to the live screen | `to_bottom_returns_to_the_live_screen` |
| Scrolling stops at the start and the end of history | `scrolling_stops_at_both_ends_of_history` |
| A key press returns the view to the bottom | `typing_returns_the_view_to_the_live_screen` |
| The scroll position is shared by everyone watching the pane | `the_scroll_position_is_shared_by_everyone_watching` |
| A client joining a scrolled pane sees the scrolled view | `a_client_joining_a_scrolled_pane_sees_the_scrolled_view` |
| New output does not move the view while history is being read | `output_arriving_while_scrolled_back_does_not_move_the_view` |
| History is there after reconnecting | `history_is_still_there_after_reconnecting` |
| History survives a server reload | `history_survives_a_reload` |
| A scrolled view stays scrolled after a reload | `a_scrolled_view_stays_scrolled_through_a_reload` |
| A line soft-wrapped from the last history row into the first screen row is still one line after a reload (made wider, it is one row) | `a_line_wrapped_across_the_history_border_stays_one_line_after_a_reload` |

## Building and publishing a release — `tests/release.rs`

| Use case | Test |
|---|---|
| A dirty tree cannot build a release; a clean release is named after the package version and commit | `a_dirty_tree_builds_no_release`, `a_release_is_named_after_the_package_version_and_the_commit` |
| Each requested platform gets its own archive and `latest`; an unknown platform is refused | `each_platform_given_is_built_with_its_own_latest`, `an_unknown_platform_is_refused` |
| Publishing requires an account ID before building, and uploads archive and checksum before `latest` with the right metadata | `publishing_without_the_account_id_does_nothing`, `publishing_uploads_the_archive_and_its_checksum_before_latest` |
| Upload-only checks each requested platform and leaves other platforms alone | `upload_only_checks_what_was_built_and_leaves_other_platforms_alone` |
| The installer (`scripts/install.sh`) is uploaded after every platform's `latest`, and not at all after a failed upload | `the_installer_is_uploaded_after_latest` |
| The updater finds and installs the archive built by `release.sh` through a local CDN | `the_layout_release_sh_writes_is_what_tilrd_update_reads` |
| Linux builds use the pinned Debian 12 image; build tools (Zig, protoc, cargo-nextest) are checksum pinned and used in CI, release builds and the container | `the_linux_build_runs_in_the_pinned_debian_12_image`, `build_tools_are_pinned_by_checksum_and_used_everywhere` |
| The Linux dev container builds the dev profile with line-tables-only debuginfo, as on the host, unless `CARGO_PROFILE_DEV_DEBUG` overrides it | `the_linux_dev_container_builds_with_line_tables_only_debuginfo` |
| CI runs every AGENTS.md gate (including MSRV and full-history secret scanning), rejects removed gates, and smoke tests both executables | `ci_runs_every_gate_of_agents_md`, `ci_smoke_tests_both_executables` |
| AGENTS.md, README, USE_CASES.md, `scripts/linux.sh` and CI name one test command, `cargo nextest run`, and none still runs the suite with `cargo test` | `the_test_command_is_the_same_everywhere` |
| Repository documentation is available at runtime with the same contents as in the build snapshot | `repository_documentation_matches_the_build_snapshot` |
| Release Please bumps the workspace and lockfile; the release builds each target, checks native versions against tag/commit, and packages both executables, licenses and README with a checksum | `release_please_bumps_the_workspace_version_and_lock`, `a_release_builds_and_packages_every_target` |
| The CDN job runs after assets and uploads only built platforms | `the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` |
| The pinned macOS SDK is selected in clean worktrees and release workflows; the Zig installer checks it before downloading | `the_pinned_zig_selects_a_compatible_macos_sdk`, `the_macos_zig_installer_checks_its_sdk_before_download` |

## Version baked at build time — `tests/build_script.rs`, `crates/tilr/tests/build_script.rs`

| Use case | Test |
|---|---|
| The build script watches only files that exist, and `HEAD` and `logs/HEAD` of the real git directory when git names them: a repeated build in a git worktree compiles nothing, a checkout still rebuilds with the new commit | `build_script::every_watched_file_exists_and_the_git_head_is_watched` (tilrd and tilr) |

## Updating from the CDN — `tests/update.rs`

| Use case | Test |
|---|---|
| `tilrd update` installs the latest release for its platform, `tilrd` and `tilr` from one archive; an explicit reload request tells a running server to reload, the shell intact | `update_installs_the_latest_release_and_the_running_server_keeps_its_shells` |
| The latest release is already installed: nothing is downloaded, the file is untouched | `update_does_nothing_when_the_latest_release_is_the_one_installed` |
| The archive does not match its checksum, does not unpack (not an archive, cut short), lacks `tilrd` or `tilr`, one of them does not run, or there is no release for the platform: nothing is installed, nothing is left next to it | `update_installs_nothing_that_fails_its_checksum_or_does_not_run` |
| The CDN misbehaves: the download breaks off halfway, a release is named but its files are not there yet, someone else's page answers instead of the CDN, nobody answers: nothing is installed | `update_installs_nothing_when_the_cdn_misbehaves` |
| The executable's directory is not writable: nothing is installed | `update_installs_nothing_when_it_cannot_write_next_to_the_executable` |
| † The updater is killed just before the swap: the executables are the old ones; the next update goes through and cleans up the unpacked release | `an_update_killed_before_the_swap_changes_nothing_and_the_next_one_tidies_up` |
| † `tilr` is already in place but `tilrd` cannot be swapped: the old `tilr` is put back (or removed if there was none) | `a_server_that_cannot_be_put_in_place_puts_the_old_client_back` |
| † Two updates at once: the release is installed once, the second waits and sees it is already there | `two_updates_at_once_install_the_release_once` |
| A server Tilr.app manages (`app-managed.json` next to the executable): `tilrd update` fails, saying so, without contacting the CDN | `app_managed_server_never_contacts_the_standalone_update_feed` |

### Live check on a machine — `scripts/update-test.sh`

The automated tests above go to a fake CDN. This protocol checks the real one:
it installs an old release on a machine, opens panes and updates to the latest.

```sh
scripts/update-test.sh <ssh-host> <old-version>
scripts/update-test.sh ubu-test 0.1.0-7b5740b
```

Needs: ssh access, `curl` and `sha256sum` on the machine; locally, a built
`test-client`. The test server lives in `~/tilr-update-test` on its own port,
leaves an installed tilr alone and cleans up after itself. One line per check,
`ok` or `not ok`; exit code 1 if anything did not match.

| # | Use case | What is checked |
|---|---|---|
| 1 | Install by hand | The old release downloads, matches its checksum, starts and names its version |
| 2 | Update from inside a pane, four panes open | The version is the latest; the server's pid is the same; one reload, with all panes; the shell is the same, its variable and background job are there; the counter printed with no gaps or pauses; the full-screen program stayed on screen and closes; the pane running a command instead of a shell is alive |
| 3 | Update again | "Already the latest", no reload |
| 4 | CDN unreachable | An error; the executable untouched, nothing left next to it, the server keeps running |
| 5 | Rollback by hand and update again, panes open | The panes survive the way back and forward; the server's pid is the same; the counter has no gaps across all three reloads; no refusals or errors in the log |
| 6 | Update with the server stopped | The update goes through; the server starts as the new version and opens a pane |

Not covered: a client connected at the moment of the update (the automated
tests check reconnecting); scrollback after an update; a release with another
state format; platforms other than `linux-aarch64`.

## Updating without losing shells — `tests/reload.rs`

| Use case | Test |
|---|---|
| Unit: a controlled launch hold exceeding ten virtual seconds does not consume an opted-in compatibility probe’s execution deadline; a hold after startup still times out; ordinary and release probes include startup in the deadline | `reload::tests::delayed_state_formats_obeys_the_selected_policy` |
| A foreground command held across a reload reports completion after it is released | `a_command_running_across_a_reload_is_reported_when_it_ends` |
| Reload on a signal: the shell is the same, the screen intact, the server keeps running | `panes_survive_a_reload_asked_for_with_a_signal` |
| Replacing the executable keeps the connection until an explicit reload request; the shells survive | `replacement_waits_for_an_explicit_request_and_keeps_the_shells` |
| Explicit reload selects only its installation, including TCP-only servers, renamed binaries and paths with spaces; Linux needs no ps | `an_explicit_request_selects_only_its_installation_without_a_socket` |
| Explicit reload with no server starts nothing, including on Linux without ps | `an_explicit_request_with_no_server_starts_nothing` |
| The obsolete auto-reload option is rejected | `the_obsolete_auto_reload_option_is_rejected` |
| A broken new executable: no reload, the server carries on with the old code | `a_broken_executable_is_not_reloaded_into` |
| `state-formats` prints this build's formats; a new executable that reads another pane state format is not reloaded into, shells intact; a suitable executable is picked up after that | `an_executable_that_reads_another_state_format_is_not_reloaded_into` |
| Unit: a successor that reads another supervisor or pane state format, cannot show it can resume (no `reload-check 1`), names no format or an unparseable one is refused with that reason; one with the same formats is accepted (`src/reload.rs`) | `reload::tests::check_formats_rejects_every_format_mismatch`, `reload::tests::an_executable_reading_the_same_formats_can_take_over` |
| After a reload the server and pane processes look the same in `ps` as before; the files with the handed-over state are read and removed | `a_reload_shows_in_no_command_line_and_leaves_no_file_behind` |
| A reload with no panes | `a_reload_with_no_panes_open_works` |
| A second server of the same user (on `<server_id>-<pid>.sock`) keeps its host socket across a reload: `tilr edit` in a pane from before reaches its clients | `a_second_server_keeps_its_host_socket_across_a_reload` |
| Several panes: each keeps its own shell and its own state | `every_pane_keeps_its_own_shell_across_a_reload` |
| Output arriving during a reload is not lost | `output_printed_during_a_reload_is_not_lost` |
| Input typed just before a reload reaches the shell, once | `input_typed_just_before_a_reload_reaches_the_shell` |
| Several reloads in a row | `a_server_can_reload_again_and_again` |
| A shell that exits after a reload reports its exit code | `a_shell_that_exits_after_a_reload_still_reports_its_exit_code` |
| A pane closed just before a reload, its program ignoring SIGHUP: the program is still killed at the close deadline, the pane exits | `a_closing_pane_is_still_killed_after_a_reload` |
| A pane's title, directory and creation time are kept | `the_title_and_directory_of_a_pane_are_kept_across_a_reload` |
| A program with a scrolling region and origin mode: after a reload the cursor is on the same cell and the next character lands next to the last one | `the_cursor_under_origin_mode_is_where_it_was_after_a_reload` |
| Stopping the server ends the shells | `stopping_the_server_ends_the_shells` |
| The same after a reload | `stopping_the_server_after_a_reload_ends_the_shells` |
| † A reload failed before the handover: everything as it was, even client connections intact | `a_reload_that_fails_before_the_handover_leaves_everything_as_it_was` |
| † A pane process could not reload: the pane carries on with the old code | `a_pane_that_cannot_hand_over_carries_on_in_the_code_it_has` |
| † A pane process crashed while taking over: that pane is lost (its shell ended), the others are intact | `a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on` |
| † The new version crashed after taking over: the server and all shells are lost (the limit of reloading, there is no way back); no orphaned shells are left | `a_version_that_crashes_taking_over_ends_the_server_and_its_shells` |
| † The server killed in the middle of a reload: the shells end, no orphans are left | `a_server_killed_in_the_middle_of_a_reload_leaves_no_shell_behind` |
| † The executable replaced again while a reload was under way: no shell is lost | `a_second_atomic_replacement_before_exec_keeps_the_shells` |
| † The new version panics resuming in its check run: no exec, the same pid, connection and shell, no state or check file left | `a_successor_that_panics_resuming_is_not_reloaded_into` |
| The new version rejects the server's command line (`--socket`): no exec, the log names the argument, the socket stays | `a_successor_that_rejects_the_command_line_is_not_reloaded_into` |
| The configuration file has a key the new version refuses: no exec; with the key gone the next reload goes through | `a_successor_that_refuses_the_configuration_is_not_reloaded_into` |
| † The check run of the new version never answers: killed after 10 s, no exec, the server served clients meanwhile | `a_successor_that_never_answers_the_check_is_not_reloaded_into` |
| † A slow check run that passes: the server serves meanwhile, then reloads with the same pid | `a_successor_is_checked_while_the_server_serves_and_then_reloaded_into` |
| † A runtime-named pane check pauses until the test releases it, and the next reload needs a fresh release | `a_runtime_named_pause_holds_a_pane_check_until_released` |
| † A paused successor whose controller disconnects fails its check; the original server carries on | `a_disconnected_pause_controller_refuses_the_reload` |
| † A pane process's check of its successor fails: the supervisor reloads, that pane carries on with the old code and its shell | `a_pane_whose_successor_fails_the_check_carries_on` |
| † SIGHUP and SIGUSR1 while the successor starts, before its handlers: held for it; SIGHUP ignored, one more reload; a shell started afterwards has no signal blocked | `signals_arriving_while_the_successor_starts_are_held_for_it` |
| † SIGTERM while the successor starts: a clean shutdown (exit 0), the shells end | `a_sigterm_while_the_successor_starts_stops_the_server_cleanly` |
| † SIGINT while the successor starts: held across the exec like SIGTERM, a clean shutdown (exit 0), the shells end, socket and lock removed | `a_sigint_while_the_successor_starts_stops_the_server_cleanly` |
| † SIGTERM right before the supervisor's exec (once failpoint `reload-exec` logs that it sleeps): no thread of the old image takes it; the successor logs `SIGTERM came during the reload` and stops cleanly (exit 0), the shell ends | `a_sigterm_while_the_supervisor_execs_waits_for_the_successor` |
| † SIGTERM after the supervisor read its events a last time, before the panes detach and the exec (failpoint `reload-after-drain`): raised again before the exec; the successor logs `SIGTERM came during the reload` and stops cleanly (exit 0), the shell ends | `a_sigterm_while_the_reload_prepares_its_exec_stops_the_server` |
| † The supervisor's exec fails in `execvp` (failpoint `reload-exec-missing`), then its stderr reader goes away: the next log line does not end it (SIGPIPE ignored again), the same connection, pane and shell | `a_broken_pipe_after_a_failed_exec_does_not_kill_the_server` |
| † SIGUSR1 every 20 ms for 3 s: several reloads, the same pid and shell | `a_storm_of_reload_signals_costs_no_shell` |

## Clipboard — `tests/clipboard.rs`

| Use case | Test |
|---|---|
| A pasted image reaches a program through Claude Code's commands, byte for byte | `a_pasted_image_reaches_a_program_that_reads_the_clipboard` |
| The clipboard is shared by all panes, the next paste replaces it | `the_clipboard_is_shared_by_all_panes_and_replaced_by_the_next_paste` |
| An upload that is too large is refused, and no old image comes back | `an_upload_over_the_limit_is_refused_and_leaves_no_stale_content` |
| A flag turns the clipboard off | `the_clipboard_can_be_turned_off` |
| Clipboard content expires | `pasted_content_expires` |
| OSC 52 printed in a pane reaches the client watching it that takes copies (`"clipboard-write"`) with the pane, `text/plain;charset=utf-8` and the text, and becomes the clipboard another pane pastes; a watcher without `"clipboard-write"` and a client watching another pane get nothing | `osc_52_in_a_pane_reaches_the_watching_client` |
| `xclip -selection clipboard -t image/png -i` in a pane: the client gets the image byte for byte, `wl-paste -t image/png` in another pane prints it; `wl-copy TEXT` and `wl-copy` from stdin copy | `xclip_in_a_pane_reaches_the_watching_client` |
| A 1 MiB copy comes in 4 chunks of one transfer; an 8 MiB copy to a client that reads nothing is cut short by the next copy and never gets its last chunk (a 9 MiB read taking turns with it in the bulk lane ends first) | `a_large_copy_comes_in_chunks_and_a_newer_one_replaces_it` |
| `--no-clipboard`: OSC 52 in a pane is not sent to a client that takes copies | `nothing_is_copied_to_clients_with_the_clipboard_turned_off` |
| `--clipboard-max 1K`: `TILR_CLIPBOARD_MAX` is 1024 in the pane; `xclip -i` of 2 KiB exits 1 with `too large: 2 KB, the clipboard takes at most 1 KB` and the clipboard keeps what it held; an OSC 52 copy of 2 KiB is dropped with a log line and not sent | `a_copy_over_the_limit_is_refused` |
| A copy no client takes: `xclip -i` exits 0 without running a real `xclip` later on `PATH`, the copy is the clipboard until `--clipboard-ttl`, and a client that subscribes later is not sent it | `a_copy_nobody_takes_stays_in_the_shared_buffer` |
| Unit: `xclip` copy requests (`-i` or no `-o`, `-t`, files, `-rmlastnl`; the primary selection is not ours) (`src/clipboard.rs`) | `clipboard::tests::xclip_copy_requests` |
| Unit: `wl-copy` requests (`-t`, `--type=`, `-n`, text arguments, `--`; `--primary` and `--clear` are not ours) (`src/clipboard.rs`) | `clipboard::tests::wl_copy_requests` |
| Unit: a copy over the limit is refused with its whole size, from text or files (`src/clipboard.rs`) | `clipboard::tests::copies_over_the_limit_are_refused` |
| Unit: OSC 52 (BEL and ST) and OSC 1337 Copy to the clipboard are reported as copies of `text/plain;charset=utf-8`; the primary selection, reads and clearing are not (`crates/tilr-screen/src/ghostty.rs`) | `ghostty::tests::osc_52_is_reported_as_a_clipboard_write` |

## Files and `tilr edit` — `tests/files.rs`

The files protocol (`Fs*`, feature `"files"`) and host events (`HostEvent`,
feature `"host-events"`): a program in a pane asks the client to open something
through the server's socket.

| Use case | Test |
|---|---|
| The client sees the `files` feature and the limit; directory listing: the path without links, entries by name, type, size, mode, time; a link is shown as a link with its target's type, a broken one as `other`; `stat` does not follow a link, the root can be looked at; listing through a link and through `..`; errors go in the answer to the request | `a_client_lists_a_directory_and_looks_at_its_entries` |
| A name that is not UTF-8 (Linux only): in a listing, a string with U+FFFD and the exact bytes in `name_bytes`, which a UTF-8 name does not have; by its bytes (`path_bytes`, `from_bytes`/`to_bytes`) the file is read, `stat` answers with the path's bytes, it is renamed and deleted | `a_name_that_is_not_utf8_is_listed_with_its_bytes_and_named_by_them` |
| A file is read in chunks, whole and in parts (offset/length); past the end: empty; more than the limit at once: refused, in parts: allowed | `a_file_is_read_in_chunks_whole_or_in_part` |
| Writing in chunks: a new file (`create`), replacing (`truncate`, the mode kept), writing through a link; already exists / no file / a directory / over the limit / less or more data than announced: refused, the file untouched; the client left mid-write: the file is as before, no temporary file | `a_file_is_written_whole_or_not_at_all` |
| A client that answers only the latest of the pings it has read (as RFC 6455 allows), with `--client-timeout 1` so liveness pings come between the bulk window's marks: a 16 MiB file still arrives whole, the bulk window does not stall | `a_client_answering_only_the_latest_ping_gets_the_whole_file` |
| A directory is created; renaming a file and a directory, over an existing one: refused; deleting a link does not touch its target; a non-empty directory is deleted only with `recursive` | `directories_are_made_entries_renamed_and_deleted` |
| Watching a directory: creation, change and deletion arrive as `FsChanged`; a save through the protocol is one change of the file, the temporary file is not seen; subdirectories are not watched; after `FsUnwatch` no changes | `a_watched_directory_reports_what_changes_in_it` |
| A watched save is held after its durable rename until the watcher queues MODIFY: the test retains that notification while awaiting the later write acknowledgement | `a_watched_save_can_be_reported_before_its_ack` (`failpoints`) |
| Nothing outside `roots`: `..`, a link leading out (listing, reading, writing, watching), a foreign path: "outside", not even existence outside is revealed; a root cannot be deleted or moved; a link inside itself can | `nothing_outside_the_shared_directories_is_served` |
| `readonly = true`: looking and reading are allowed; write, mkdir, rename, delete are refused, nothing changes on disk | `a_read_only_server_shows_files_but_changes_nothing` |
| `--no-files` and `[files] enabled = false`: no feature, `fs_max_bytes` 0, requests get no answer and have no effect, the connection is alive | `files_can_be_turned_off` |
| In a pane: `$TILR_PANE`, `$TILR_SERVER` (the server id) and `$TILR_SOCK` (the socket); `tilr edit notes.md` reaches every client watching the pane as a `HostEvent` with an absolute path; `tilr open URL -- args`, `--kind`; other `tilr` commands go to the next `tilr` on PATH | `tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file` |
| Nobody watching the pane, or only a client that did not say `"host-events"` in `Hello.features` (one built before them, which is not counted as a recipient): "no tilr client attached", exit code 1; outside a pane: "not inside a tilr pane", exit code 1; a foreign pane: "no such pane"; non-JSON: `bad request`; a stopped server removes the socket | `tilr_edit_fails_with_no_client_watching_or_outside_a_pane` |
| One user's two servers (one state directory, so one server id) each keep a host socket: the first `<server_id>.sock`, the second `<server_id>-<pid>.sock`; `tilr edit` in a pane of each reaches its own clients, and stopping the second leaves the first one's socket | `two_servers_of_one_user_keep_their_host_sockets` |
| A second server of the same user starting (three times) never takes `$TILR_SHIMS/tilr` away from the first one's pane, `tilr edit` there still works, and the first server logs no refused connection for the second one's probe of its host socket | `a_second_server_starting_leaves_the_first_ones_panes_their_tools` |
| Unit: links in `bin/` replaced over and over (two executables in turn) are never missing for a reader, nothing is left beside them, a right one is not touched and one left out is removed (`src/clipboard.rs`) | `clipboard::tests::the_shims_are_never_missing_while_another_server_installs_them` |

## SOCKS proxy — `tests/socks.rs`

| Use case | Test |
|---|---|
| A page over the WebSocket `/socks` with the token in `Authorization: Bearer` reaches a service on the server's loopback (IPv4), there and back, more than one message included | `a_page_reaches_a_loopback_service_through_the_socks_websocket` |
| The token in `?token=`; the name `localhost` is resolved on the server | `a_page_that_cannot_set_headers_gives_the_token_in_the_query` |
| `pb.localhost`, `nonexistent.LocalHost`: the server's loopback, no DNS | `any_name_under_localhost_is_the_servers_loopback` |
| No token or someone else's: 401; a server with `--no-auth` asks for no token | `the_socks_websocket_needs_the_access_token` |
| A page from a foreign origin: 403, as for the main protocol | `the_socks_websocket_comes_only_from_an_allowed_origin` |
| Without `--socks` there is no proxy: `/socks` is 404, raw SOCKS is disconnected, the main protocol as before | `the_proxy_is_off_unless_asked_for` |
| `socks = true` in the configuration file turns the proxy on | `the_configuration_file_turns_the_proxy_on` |
| Raw SOCKS5 on the same port: the password is the token; a wrong password: 0x01; no password: 0xFF | `a_raw_socks_client_gives_the_token_as_its_password` |
| Raw SOCKS5 on a server with `--no-auth` asks for no password | `a_raw_socks_client_of_a_server_without_authentication_needs_no_password` |
| BIND: 0x07; a closed port: 0x05, then the WebSocket is closed with 1000 | `only_connect_is_served_and_a_refusal_is_reported` |
| A text message closes the WebSocket with 1003 | `a_text_message_closes_the_socks_websocket` |
| Several sessions run in parallel, one that never finishes its handshake holds no one up, the terminal works | `socks_sessions_run_side_by_side_with_the_terminal` |
| A raw client that sends its greeting but no request is closed when the shared test clock advances by ten seconds | `a_session_that_does_not_say_where_it_goes_is_closed` (`failpoints`) |

## SSH agent — `tests/agent.rs`

Agent forwarding as with `ssh -A` (feature `"agent"`): every pane has its own
socket in `SSH_AUTH_SOCK`, and a program talks through it to the agent on the
client's side in a `StreamOpen` / `StreamData` / `StreamClose` stream.

| Use case | Test |
|---|---|
| The server announces the `agent` feature; with `--no-agent` and `agent = false` it does not, there is no socket, `SSH_AUTH_SOCK` in the pane is empty | `the_agent_is_forwarded_unless_turned_off` |
| In a pane `SSH_AUTH_SOCK` points at its own socket (`agent/<pid>/<pane>.sock`, 0600, directory 0700); every pane has its own; the pane closed: no socket | `a_pane_has_ssh_auth_sock_pointing_at_its_own_socket` |
| A program connected: the client gets `StreamOpen` (`ssh-agent`, the pane); bytes go both ways, more than one message too; the program closed: `StreamClose`; the client closed: the program reads what was sent and gets the end; data for a closed or foreign stream is ignored | `a_program_talks_to_the_agent_of_the_client_through_the_panes_socket` |
| The pane is watched by a client without `agent`, the client with `agent` does not watch it or unsubscribed: the connection is closed at once, the clients get nothing | `with_no_agent_client_attached_the_program_finds_no_agent` |
| The agent client disconnected mid-stream: the program reads the end; the next client serves the same socket | `the_program_reads_the_end_when_the_agent_client_disconnects` |
| Two clients with `agent`: the stream goes to the one that subscribed last (a client without `agent` does not count); it unsubscribed: the first gets it | `the_client_that_subscribed_last_serves_the_agent` |
| Reload: an open stream ends, the socket at the same path accepts again and leads to the reconnected client | `the_panes_socket_works_after_a_reload` |
| Agent sockets left by a server killed with SIGKILL are removed when the next one starts; the live sockets of another server stay | `the_sockets_a_killed_server_left_are_removed_when_the_next_starts` |
| A second server of the user starts next to a live one: it does not connect to the live one's agent sockets (no stream for its client, no refused connection logged) nor remove them | `a_server_starting_leaves_the_agent_sockets_of_a_live_one_alone` |

## Exec — `tests/exec.rs`

A program on the server's machine without a pane and without a shell (feature
`"exec"`): `ExecStart` → `ExecStarted`, output in `ExecOutput` (1 is stdout, 2
stderr), `ExecExit` at the end; `ExecInput` (acknowledged by `ExecInputAck`),
`ExecSignal`, `ExecCancel`. The program leads a process group of its own, and
nothing in the group outlives the exec.

| Use case | Test |
|---|---|
| The server announces the `exec` feature; with `--no-exec` and `exec = false` it does not, `ExecStart` is answered with `ExecStarted` `exec_id` 0 and error `exec is turned off`, the connection is alive | `exec_is_offered_unless_turned_off` |
| stdout and stderr apart, the exit code; arguments as given (`$HOME`, `*`, `` `id` ``, a space are not expanded); the directory and the environment, without a directory the home one; without stdin the program reads the end; no program, an empty `argv`, no directory, an environment entry that is not `NAME=value`: refused in `ExecStarted.error`; the start, the end and the refusal are logged | `a_program_runs_without_a_shell_and_its_outputs_come_apart` |
| An exec does not get the service manager's variables (`NOTIFY_SOCKET`, `LISTEN_*`, `WATCHDOG_*`) the server was started with; its own `env` may still set them | `an_exec_does_not_get_the_service_managers_variables` |
| Killed by a signal (its own SIGTERM, a SIGINT from the client to the group): `signal`, code 0 | `the_signal_that_killed_a_program_is_reported` |
| A time limit of 0.5 s: `timed_out`, SIGKILL, under 3 s; neither the program nor its background child is left | `a_time_limit_kills_the_program_and_its_children` |
| `ExecCancel`: `cancelled`, the program and its child killed at once | `a_cancelled_program_ends_at_once_with_its_children` |
| The program exited, leaving a background process in its group that holds stdout: the exec ends with it, the process is killed | `what_a_program_leaves_running_in_its_group_goes_when_it_exits` |
| The program exited, leaving a process outside its group (`setsid`) that writes to stdout without pause (`yes`): `ExecExit` within 3 s; its slot is free, 64 execs start afterwards | `a_process_outside_the_group_that_keeps_writing_does_not_hold_the_exec` |
| The shell exited, a process outside its group holds stdout and writes now and then: `ExecCancel` ends the exec within 1 s, `cancelled` | `a_cancel_during_the_drain_ends_the_exec_at_once` |
| stdin: empty and closed is the end of file; 8 MiB in chunks within the window by acknowledgements arrive whole; a client more than 4 MiB ahead of a program that does not read: the exec is killed, `reason` `input overrun` | `stdin_reaches_the_program_and_is_acknowledged` |
| 32 MiB of output: while the client does not read, the program waits (has not finished writing); then everything whole and in order, stderr apart, the exit after the output | `a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client` |
| Eight 1 s execs together take about a second, each with its own output; a pane works alongside | `execs_run_side_by_side_with_the_panes` |
| The client left: its programs and their children are killed, `client gone` in the log | `a_client_that_goes_takes_its_programs_with_it` |
| Reload: a running exec is killed with its group, the client gets `ExecExit` with `reason` `reload` and the output before it before the connection closes; after reconnecting exec works | `a_reload_ends_the_programs_and_their_clients_hear_why` |
| Reload while the client reads none of an exec's output (`cat /dev/urandom`, its lane full): the server gives up waiting for the exec; once the client reads again, the last message of that exec is `ExecExit` `reason` `reload`, no output after it | `an_exec_given_up_at_a_reload_ends_with_its_exec_exit` |

## Metrics — `tests/metrics.rs`

The client measures input latency itself, without synchronising clocks: it
tags input with `ClientMessage.seq` = `HelloAck.seq_base` + n and waits for the
frame whose `Frame.last_input_seq` is that number (the last credited input,
from any client); `Frame.timing` in the frame is the time spent inside the
pane process. The server's counters are at `GET /metrics`.

| Use case | Test |
|---|---|
| A fixed test clock assigns distinct parse/build/encode durations; parse accumulates across two invisible reads and a visible read, and changing the configuration leaves encode reporting the previous frame | `frame_stage_durations_are_controlled_per_operation` |
| Before any input `last_input_seq` = 0; frames after input with `seq` carry it, `timing` is always there, `build_us` > 0, a frame with output has `output_bytes` and `reads`; input without `seq` does not disturb the number; a `Scroll` with `seq` that moved nothing gets its own frame; a client numbering from 1 without `seq_base` works | `a_frame_names_the_input_it_shows_and_how_it_was_made` |
| A no-op scroll while output is accumulated: one frame carries the scroll seq and the changed output; after the final prompt, another no-op scroll gets an empty frame | `a_no_op_scroll_frame_includes_accumulated_output` |
| Input that changed nothing on screen with no pending output (a click without mouse tracking, an empty `RawInput`): a frame with no rows carrying its `seq`, the same cursor, size and modes; input without `seq` gives no frame | `input_that_changes_nothing_is_answered_with_an_empty_frame` |
| Reconnecting (a page reload): the new `seq_base` is greater than the old one; the full frame carries the old connection's number, not its own; input of the new one carries its number | `a_client_that_reconnects_tells_its_input_from_before` |
| Two clients on one pane: each has its own `seq_base`, both see the numbers of both, in order | `two_clients_on_one_pane_each_see_their_own_input` |
| `Scroll` is credited at once, although keyboard input is still waiting for the pty (1 MiB in a pty with no reader) | `a_scroll_is_credited_while_key_input_waits_for_the_pty` |
| Server reload: `last_input_seq` and input waiting for the pty survive it, the input is credited after, `input_us` counts from receipt to the reload; connection numbers carry on | `the_last_input_and_input_still_waiting_survive_a_reload` |
| `/metrics` with no token, with a wrong one (in `?token=` and in `Authorization`): 401; a foreign origin: 403; with the token either way: 200, `text/plain; version=0.0.4`; the main protocol on the same port as before | `metrics_ask_for_the_access_token` |
| A server with `--no-auth` asks for no token for `/metrics` when `Host` is localhost, `127.0.0.1`, `[::1]` (with or without a port) and `Sec-Fetch-Site` is not cross-site | `metrics_need_no_token_on_a_server_without_one` |
| Without a token: `Sec-Fetch-Site: cross-site`, a foreign `Host` (DNS rebinding) or no `Host`: 403; with a token, the token decides | `without_a_token_metrics_are_for_this_machine_only` |
| A request that arrives in pieces (the request line and the empty line split) | `a_metrics_request_may_come_in_pieces` |
| After input and frames: the pane's frame, byte and input counters, the routing and loop-lag histograms, CPU and memory of the supervisor and the pane process | `metrics_count_frames_input_and_routing` |
| An old connection superseded by the same client is counted in `tilrd_client_drops_total{reason="superseded"}` | `metrics_count_the_clients_let_go_and_why` |
| A wrong-token Hello is refused without a drop | `metrics_count_the_clients_let_go_and_why` |
| A lagging client (a small receive buffer, not reading, a stream of output in the pane): its frames are skipped (`tilrd_pane_frames_skipped_total`); once it has read up it gets a full frame and the end of the output; its input, whose echo was in a skipped frame, arrives in the full frame's `last_input_seq`; it is not dropped | `a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo` |

## Test infrastructure — `tests/cleanup_stand.rs`

| Case | Check |
|---|---|
| After a test-owned server sends `READY=1`, interrupt its test by SIGTERM and by a controlled timeout; check its exact PID and macOS `pgrep`, report survivors, and clean the owned process group even on failure | `cleanup_stand::server_ready_then_wait_for_interruption` (ignored; run by `scripts/test_server_cleanup_stand.py`), `scripts/test_server_cleanup_stand_selftest.py` |
| Test-process panic (including before startup output is parsed), SIGTERM and forced timeout stop owned supervisors, panes and shells, for original and copied executables | `cleanup_stand::owned_servers_exit_after_panic_signal_and_timeout` |
| A panicking client test stops its copied daemon even after its command socket is removed | `client::a_panicking_test_stops_its_copied_daemon_without_a_command_socket` |

## Not covered

- `tilr` and a socket served by another uid (refused before a byte is sent): it
  takes a second user, so it is itests UC-03A (and UC-034); the directory
  checks themselves are unit tests in `tilr-proto` (`local.rs`), the token
  that goes to loopback targets only one in `tilr` (`target.rs`).
- Client: the web client's behaviour on a dropped connection (automatic
  reconnect, back into the same pane) is checked by hand only. The web client
  does not send `Ping` yet.
- A slow client that the server drops for a full queue (1024 packets): with
  frames skipped, only a stream of packets other than frames fills it;
  end-to-end slow-network use cases are itests UC-043, UC-049.
- A reload while a full-screen application (vim, htop) is open in a pane:
  there are unit tests of the screen snapshot, no end-to-end use case. The
  unit tests also cover a cursor waiting to wrap at the last column, the
  cursor and left/right margins under origin mode, and kitty images whose top
  row is in the history (`ghostty::tests`), which no end-to-end use case
  checks.
- A reload while a pane is being created.
- An update when the disk is full: here only a non-writable directory is
  checked, and that test does not run as root (in a container); a full disk
  while downloading and while writing state is itests UC-065. That a state
  file is written whole or not at all (a temporary next to it, a rename), and
  that a failed write leaves nothing, is checked by the unit tests in
  `reload.rs`.
- The new version crashes after the handover, having passed its check run:
  the server and shells are lost (the test pins this down). There is no
  protection beyond the checks before the exec (`state-formats` and the check
  run of the resume).
- An update between versions with different state formats without a restart:
  such a reload is currently refused (shells intact); there is no migration of
  state from the old format to the new one.
- Keys and mouse: encoding is checked by the screen's unit tests (including
  Ctrl on a Russian layout by the physical key `KeyInput.code`, Alt as `ESC`
  and the character produced, composed characters (AltGr, Option) as
  themselves, Dvorak and AZERTY, where the Latin character decides, and the
  base key in the kitty protocol); end-to-end use cases are itests UC-079.
- Scrolling while the pane is resized.
- tilr Cloud: automated tests play Cloud and `cloudflared`. A real
  `cloudflared` and a real Cloudflare tunnel have never been tested.
- A `cloudflared` left behind after the server crashed (SIGKILL) is not
  cleaned up by anyone.
- Files: the temporary file of a write cut short by a reload is removed by
  the new server as it starts (by its note in `writes/` of the runtime
  directory; a unit test in `files.rs`, end to end itests UC-064); the client
  sets up watches again after a reload, as it does subscriptions.
- Files and updates on disk: that a finished write and an installed release
  are synced (`fsync` of the file before the rename, of the directory after
  it, before the answer or `updated`) is not visible to these tests; itests
  UC-046 and UC-060 trace the system calls under `strace`. A power loss is not
  tested.
- Login shells and their startup files: which shell is started how (zsh
  through `ZDOTDIR`, bash `--posix` with `$ENV`, fish `--init-command`, others
  as before) and the wrappers in `<runtime dir>/shell/` are unit tests
  (`pty::tests::a_login_shell_is_wrapped_by_its_name`,
  `shell::tests::the_wrappers_are_written_private_and_replaced_whole`,
  `shell::tests::no_wrappers_means_shells_start_as_before`,
  `shell::tests::servers_starting_at_once_each_install_the_wrappers`); that the tools
  stay first on `PATH` with profiles that reset it, in real bash, zsh, fish
  and dash, is itests UC-07B.
- `tilr service install|uninstall`: `crates/tilr/tests` start no service
  manager. Unit tests in `crates/tilr/src/service.rs` check the unit, plist
  and crontab line, that a unit or plist without the `tilr service` mark is
  not replaced (`a_file_without_the_mark_is_not_replaced`) and that a missing
  crontab is recognized from cronie's and busybox's messages
  (`a_missing_crontab_is_recognized_from_cron_and_busybox`); end to end with
  stand-in `systemctl`, `loginctl` and `crontab`: itests UC-020…UC-023.
- `tilr edit` / `tilr open` and a host socket another user serves at
  `$TILR_SOCK` (exit 1, not a byte sent): it takes a second user, so it is
  itests UC-03A; the uid comparison is the unit test
  `local::tests::a_peer_of_another_uid_is_refused` in `tilr-proto`.
- `tilr` in a terminal: drawing is checked only by what shows in the pty
  (text, the prompt); colours, the mouse, pasting and layouts have not been
  run through a real terminal. Drawing assumes the pane is the size of the
  terminal (a second client of another size changes it).
- SSH agent: pane sockets left behind after the server crashed (SIGKILL) are
  not cleaned up by anyone. A real `ssh` with a real agent through the client
  has not been tested: the tests play both the program and the client.

Shared Ghostty build tooling: `ghostty_script::shared_ghostty_build_reuses_complete_toolchain_prefixes`
checks pinned build flags, package relocation, reuse, Zig/SDK identity changes, Linux
SDK independence and explicit recovery after failure through deterministic tool fixtures.
