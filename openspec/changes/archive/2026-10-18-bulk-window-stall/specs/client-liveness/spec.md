## MODIFIED Requirements

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
