# client-liveness Specification

## Purpose

Defines how `tilrd` tells that a client is still there and keeping up, and what it does when one is not: WebSocket pings, silent and deaf clients, the close codes 4000 / 4001 / 4002, superseding a client's older connection by `Hello.client_id`, the viewers a pane counts, the two lanes of a client's queue, and skipping frames for a client that lags. Metric names are in the `metrics` spec, what a frame holds in `screen-frames`, panes themselves in `panes`, the `tilr` client's own reconnect loop in `tilr-cli`.

## Requirements

### Requirement: The server pings its clients
On every main-protocol connection the server SHALL send a WebSocket ping every third of `--client-timeout` (default 30 s, so every 10 s). With `--client-timeout 0` it MUST send no such pings and drop no client for silence. A client MUST answer the server's pings, in the order it reads, as browsers and WebSocket libraries do on their own. A client that only reads its connection and sends nothing MUST stay connected.

**Verified by**: `reconnect::an_idle_client_that_is_still_there_stays_connected` and `reconnect::a_silent_client_is_kept_when_the_timeout_is_off`.

#### Scenario: Idle but there
- **WHEN** a client of a server with `--client-timeout 1` sends nothing for 3 s but keeps reading
- **THEN** its connection stays open and the log has no "client went silent"

#### Scenario: Timeout off
- **WHEN** a client of a server with `--client-timeout 0` stops reading for 2 s
- **THEN** the pane still counts it as a viewer and it can run commands afterwards

### Requirement: A silent client is dropped
When nothing at all (no message, no pong) has come from a client for the whole client timeout, the server SHALL drop it: log "client went silent", close the connection with WebSocket close code 4002 and reason `timeout`, remove it from the viewers of its panes and push the updated `PaneList` to the other clients. This MUST also apply to a connection that finished the WebSocket handshake and never sent `Hello`.

**Verified by**: `reconnect::a_client_that_vanishes_without_closing_is_dropped`, the unit test `a_client_is_judged_silent_or_deaf_by_what_it_sends_and_answers` in crates/tilrd/src/supervisor.rs, and itests UC-040 (`uc_040.py::a_frozen_connection_is_dropped_by_both_sides_and_the_client_returns_to_the_same_pane`) and UC-039 (`uc_039.py::a_websocket_that_never_says_hello_is_dropped_by_the_client_timeout`).

#### Scenario: Network drops out without a close
- **WHEN** a subscribed client of a server with `--client-timeout 1` stops reading and sending, its connection left open
- **THEN** within the timeout the other client receives a `PaneList` whose pane has `viewers` 1, and the log says "client went silent"
- **AND** the client, back on the network, connects again and finds its pane

#### Scenario: WebSocket without Hello
- **WHEN** a client completes the WebSocket handshake and then sends nothing (no `Hello`, no pongs) to a server with `--client-timeout 4`
- **THEN** the server closes the connection within about 4 s and keeps running

### Requirement: A client that no longer hears the server is dropped
The server SHALL drop a client that still sends (something within the last ping interval) but has answered no ping for the whole client timeout: the way from the server to it is cut. What the client sends, `Ping` included, MUST NOT keep such a connection alive. The log MUST say "client stopped answering pings" (not "client went silent") and the close MUST use code 4002 `timeout`. A client that stopped sending only recently MUST be judged silent at the end of its timeout, not deaf earlier.

**Verified by**: `reconnect::a_client_that_still_sends_but_no_longer_hears_the_server_is_dropped`, the unit test `a_client_is_judged_silent_or_deaf_by_what_it_sends_and_answers` in crates/tilrd/src/supervisor.rs, and itests UC-041 (`uc_041.py::a_client_that_still_talks_but_no_longer_hears_is_dropped_by_the_server`, `uc_041.py::a_cut_of_server_to_client_only_is_noticed_by_the_client_and_both_sides_let_go`, `uc_041.py::a_cut_of_client_to_server_only_is_noticed_by_the_server_and_by_a_client_that_counts_pongs`).

#### Scenario: Pings out, nothing in
- **WHEN** a subscribed client of a server with `--client-timeout 1` sends a `Ping` every 100 ms but reads nothing
- **THEN** it is dropped, the other client sees `viewers` 1, and the log says "client stopped answering pings" and not "client went silent"

### Requirement: A dropped connection really ends
When the server closes a connection on its own (timeout, too slow, superseded) it SHALL send the close frame at once, without waiting for what is still queued, give it 1 s to go out and the client 1 s to answer it; when the close frame cannot be written in that time, the server MUST reset the connection (TCP RST, linger 0) rather than leave stale data in the socket for a client that does not read.

**Verified by**: itests UC-049 (`uc_049.py::a_client_too_slow_even_for_the_latest_screen_is_dropped_and_its_connection_ends`).

#### Scenario: A client too slow to take even the close
- **WHEN** a client behind a 1 KB/s link watches a pane under an output storm, with `--client-timeout 6`
- **THEN** the server drops it within the timeout (logging "client stopped answering pings" or "client too slow"), the client sees its connection end within 15 s, and the server's memory stays bounded

### Requirement: A connection that ends is forgotten at once
When a client closes its connection or it is reset (by the client, its kernel or the network), the server SHALL forget the client at once: remove it from the viewers of every pane and push the updated `PaneList`, without waiting for the client timeout. Repeated reconnects MUST leave exactly one viewer per open connection.

**Verified by**: `reconnect::many_reconnects_leave_no_viewers_behind` and itests UC-045 (`uc_045.py::a_reset_in_the_middle_of_a_frame_makes_the_client_reconnect_at_once`, `uc_045.py::a_socket_reset_by_the_clients_kernel_is_cleaned_up_by_the_server_at_once`) and UC-050 (`uc_050.py::a_thousand_resets_leave_one_viewer_and_flat_server_memory_and_descriptors`).

#### Scenario: Five reconnects
- **WHEN** a client drops its connection and connects and subscribes again five times
- **THEN** the pane's `viewers` settles at 1

#### Scenario: Reset by the client's kernel
- **WHEN** the client's socket is destroyed (`ss -K`) while the server runs with the default 30 s timeout
- **THEN** within 3 s the pane's viewers no longer count it

### Requirement: Clients ping the server with Ping
For a client that cannot send WebSocket pings (a browser page), the server SHALL offer feature `"ping"` and answer every `Ping { nonce }` at once with `Pong` carrying the same `nonce`. A client that pings and gets no `Pong` in time MAY take the server as out of reach and reconnect.

**Verified by**: `reconnect::a_client_can_tell_that_the_server_vanished_without_closing` and itests UC-041 (`uc_041.py::a_cut_of_client_to_server_only_is_noticed_by_the_server_and_by_a_client_that_counts_pongs`).

#### Scenario: Server frozen and back
- **WHEN** a client sends `Ping { nonce: 2 }` while the server is stopped (SIGSTOP)
- **THEN** no `Pong` comes within 1 s
- **AND** after SIGCONT the `Pong` with nonce 2 arrives and the pane works as before

### Requirement: A client's newer connection supersedes its older one
A client instance SHALL name itself in `Hello.client_id` (a UUID v4 made when it starts, the same on every connection). When a `Hello` names the `client_id` of another greeted connection under the same token and on the same transport (Unix or TCP), the server MUST close the older one at once, before the new one gets `HelloAck`, with close code 4000 `superseded`, logging "client superseded by a newer connection". An empty `client_id` MUST neither supersede nor be superseded.

**Verified by**: `reconnect::a_client_back_on_a_new_connection_supersedes_its_old_one_at_once`, `metrics::metrics_count_the_clients_let_go_and_why` and itests UC-042 (`uc_042.py::a_client_back_from_a_new_ip_leaves_no_ghost_of_its_half_open_connection`, `uc_042.py::a_client_back_from_a_new_ip_leaves_no_ghost_when_its_old_path_froze_in_a_proxy`), UC-044 (`uc_044.py::a_revived_connection_left_open_by_the_client_was_superseded_by_its_reconnect`, `uc_044.py::a_revived_connection_the_client_closed_ends_at_once_leaving_one_live_connection`) and UC-052 (`uc_052.py::two_tabs_of_one_host_with_client_ids_of_their_own_both_stay_live`).

#### Scenario: Back on a new connection
- **WHEN** a client with `client_id` A watches a pane and connects again with `client_id` A while the old connection is still open
- **THEN** the old connection receives a close frame with code 4000 and reason `superseded`
- **AND** the new connection sees the pane with one viewer fewer, and a tab with `client_id` B keeps working

#### Scenario: Back from a new address
- **WHEN** the old path of a client freezes half-open and the client reconnects from another IP with the same `client_id`
- **THEN** the old connection is gone within 1 s, not after the client timeout, and no ghost viewer is left

### Requirement: Another token supersedes nobody
A `Hello` whose token differs from the older connection's SHALL NOT supersede it, whatever `client_id` it names. On a server with a token such a `Hello` is refused ("wrong access token") and the older connection stays; on a server started with `--no-auth`, a connection with another token is another client and both stay.

**Verified by**: `reconnect::the_same_client_id_under_another_token_supersedes_nobody` and itests UC-052 (`uc_052.py::the_same_client_id_under_another_token_supersedes_nobody`).

#### Scenario: Spoofed client id
- **WHEN** a connection says `Hello` with an existing connection's `client_id` and a wrong token
- **THEN** it receives `Error` "wrong access token", and the existing connection keeps running commands with `viewers` 1

#### Scenario: No-auth server
- **WHEN** two connections to a `--no-auth` server name the same `client_id` with tokens "one" and "two"
- **THEN** both stay connected, the pane counts 2 viewers and the log has no "superseded"

### Requirement: A new connection starts afresh
A connection SHALL carry nothing over from an earlier one of the same client: it subscribes again like any other, its first frame of a pane after `Subscribe` is full, and no input is replayed. Input sent into a connection that broke MAY be lost and MUST never arrive twice. A client whose connection was closed with 4000 `superseded` MUST NOT reconnect that connection.

**Verified by**: `reconnect::a_client_that_reconnects_finds_its_pane_as_it_left_it`, `reconnect::subscribing_again_sends_the_whole_screen_again` and itests UC-048 (`uc_048.py::input_cut_by_a_reset_mid_burst_is_lost_or_kept_in_order_but_never_doubled`, `uc_048.py::input_typed_into_a_silent_connection_is_lost_but_never_doubled`, `uc_048.py::input_held_in_a_connection_that_revives_arrives_once_at_most`) and UC-052 (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`); for `tilr`, `client::an_attach_superseded_by_its_own_client_id_ends`.

#### Scenario: Reconnect
- **WHEN** a client reconnects and subscribes to the pane it watched
- **THEN** its first frame is full and the shell is the same process as before

#### Scenario: Input at the moment of a cut
- **WHEN** a burst of input lines is cut by a reset and the client reconnects
- **THEN** each line has run at most once, in order

### Requirement: Panes say who watches them
`PaneInfo.viewers` SHALL be the number of connections subscribed to the pane, and `PaneInfo.viewed_by` one `Viewer` per such connection, in no particular order: `client_id` (`Hello.client_id`, empty if none), `client_name` (`Hello.client_name`) and `address` (the peer as the server sees it, `ip:port` on TCP, `unix` or `unix (pid N)` on the Unix socket). The log MUST name a client's `client_id` and address on hello, supersede and drop.

**Verified by**: `reconnect::a_client_back_on_a_new_connection_supersedes_its_old_one_at_once` and itests UC-052 (`uc_052.py::two_tabs_of_one_host_with_client_ids_of_their_own_both_stay_live`).

#### Scenario: Three viewers
- **WHEN** a client without `client_id` and two with ids A and B watch one pane over loopback TCP
- **THEN** `viewed_by` holds the ids `""`, A and B, each with an `address` starting `127.0.0.1:`

### Requirement: Frames go ahead of data in quantity
Each client's queue SHALL have an interactive lane (frames, answers, acknowledgements) and a bulk lane (`FsData`, `StreamData` then `StreamClose`, `ExecOutput` then `ExecExit`). The bulk lane MUST be written only when the interactive one is empty, so a frame waits behind at most one bulk packet. The bulk lane MUST hold at most 4 MiB per client: a producer with more waits for room, so a slow client holds it back and MUST NOT be dropped or buffered for.

**Verified by**: `exec::a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client`, unit tests `interactive_packets_go_ahead_of_bulk_ones` and `bulk_waits_for_room_counted_in_bytes` in crates/tilrd/src/outbox.rs, and itests UC-072 (`uc_072.py::parallel_transfers_both_ways_leave_the_terminal_responsive_and_the_files_intact`, `uc_072.py::parallel_reads_leave_the_terminal_responsive`) and UC-073 (`uc_073.py::a_slow_client_holds_the_program_back_and_is_neither_dropped_nor_buffered_for`).

#### Scenario: Typing during file reads
- **WHEN** a client reads several large files while typing into a pane
- **THEN** the echo of its keystrokes is not held behind the file data, and the files arrive intact

#### Scenario: Slow reader of a stream
- **WHEN** a program writes to a stream faster than the client reads it
- **THEN** the program is held back, the client stays connected, and the server's memory does not grow with the backlog

### Requirement: Bulk data in the network is bounded by pongs
The server SHALL keep at most 1 MiB of bulk data written to a client and not yet read by it. Every WebSocket ping it sends (one per 128 KiB of bulk data, one when the window fills, and the liveness pings) MUST carry `tilr-bulk:` and the bulk byte position as a big-endian u64; a pong's position moves the window, capped at the bytes written. A client that answers no pings MUST stall its bulk data (file reads, streams, execs) while its frames still flow.

**Verified by**: the unit tests `the_bulk_window_closes_until_the_client_has_read_its_marks` and `a_pong_beyond_what_was_sent_opens_no_more_than_the_window` in crates/tilrd/src/outbox.rs, `files::a_client_answering_only_the_latest_ping_gets_the_whole_file`, and itests UC-072 (`uc_072.py::parallel_reads_leave_the_terminal_responsive`).

#### Scenario: Window full
- **WHEN** 1 MiB of bulk data is written to a client that has answered none of the marking pings
- **THEN** no more bulk data goes to it until a pong carries a later position

#### Scenario: Client answers only the newest ping
- **WHEN** a client of a server with `--client-timeout 1` reads a 16 MiB file and answers only the most recent of the pings it has read
- **THEN** the whole file arrives

#### Scenario: Pong ahead of the data
- **WHEN** a client answers a marking ping with a position beyond the bytes written to it
- **THEN** the server writes it at most 1 MiB of bulk data more before a later pong

### Requirement: A lagging client skips to the latest screen
The server SHALL set `TCP_NOTSENT_LOWAT` to 4 KiB on TCP clients. A client whose interactive lane has held a packet over 250 ms lags: its frames of a pane MUST be skipped (see `metrics`) until its queue is written out, then the pane MUST be asked for a full frame (also when the queue emptied before the lag was recorded), sent to all its viewers, with `last_input_seq` the client's latest seq a skipped frame echoed. A lagging client MUST NOT be dropped for it, nor slow other clients.

**Verified by**: `metrics::a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo`, `reconnect::a_lagging_client_gets_the_last_screen_without_new_output`, the unit test `a_packet_held_too_long_is_lag_and_an_empty_queue_is_reported_once` in crates/tilrd/src/outbox.rs, and itests UC-049 (`uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others`) and UC-043 (`uc_043.py::a_lossy_slow_network_slows_the_session_without_tearing_it_down_or_hanging_it`, `uc_043.py::a_session_through_a_lossy_slow_proxy_keeps_moving_without_a_disconnect`).

#### Scenario: Small receive buffer under heavy output
- **WHEN** a client with a 4 KiB receive buffer stops reading while a pane prints 30 MB, sends input with seq S meanwhile, and then reads again
- **THEN** it receives a full frame after the lag, a frame with `last_input_seq` S, and the end of the output
- **AND** it received fewer frames than the pane produced, and stayed connected

#### Scenario: Output stops during lag
- **WHEN** a client lags while a pane prints and the pane then goes quiet
- **THEN** within 1 s of its queue being written out the client receives a full frame matching the pane's final screen

#### Scenario: Others not slowed
- **WHEN** one client is limited to 10 KB/s while another watches the same pane and types
- **THEN** the fast client's input latency stays low and the server's memory stays bounded

### Requirement: A client whose queue fills is dropped
When a client's interactive lane holds 1024 packets and another must be queued, the server SHALL drop the client: log "client too slow", close the connection at once with close code 4001 and reason `too slow`, whatever is still queued (a client that lost a diff has a corrupt screen). A client whose connection is already gone MUST be forgotten as disconnected instead.

**Verified by**: itests UC-049 (`uc_049.py::a_client_too_slow_even_for_the_latest_screen_is_dropped_and_its_connection_ends`, which accepts either "client too slow" or "client stopped answering pings").

#### Scenario: Queue full
- **WHEN** 1024 packets are queued for a client that does not read and one more is sent to it
- **THEN** the connection is closed with code 4001 `too slow` and the pane no longer counts it

## Notes

- Requirements without a test: none. Partly untested: the 4001 `too slow` close of "A client whose queue fills is dropped" (no cargo test reaches it; frame skipping keeps the queue short, and UC-049 passes on either close reason); the 1 s close timeout and the reset of "A dropped connection really ends" (UC-049 checks only that the connection ends); the `TCP_NOTSENT_LOWAT` value.
- UC-041 and UC-049 were red earlier; at HEAD itests/USE_CASES.md marks all of group E (UC-040..052) implemented with no FAIL, so no case cited here is known to be failing.
- Unit tests in crates/tilrd/src/outbox.rs and crates/tilrd/src/supervisor.rs are cited by bare name (they are not in a `tests/` file).
- The `tilr` client's keepalive and reconnect loop (pings every 5 s, 15 s of silence is a lost connection, the first retry 0.1 s after the loss, then waits doubling to 2 s; a 4000 close ends the attach) is in the `tilr-cli` spec.
