# metrics Specification

## Purpose

Covers what the server lets one measure: input latency end to end (`ClientMessage.seq`, `HelloAck.seq_base`, `Frame.last_input_seq`, `Frame.timing`) and the `GET /metrics` endpoint with its metric names, including clients let go by reason and frames skipped for a lagging client. When a client counts as lagging, the outbox lanes and the close codes are in the `client-liveness` spec; the access token itself in `access-control`; what frames contain in `screen-frames`.

## Requirements

### Requirement: Each connection has its own seq base
`HelloAck.seq_base` SHALL be different for every connection the server accepts: the connection's number in the high 32 bits (`n << 32`, `n` >= 1), numbers going on across a reload.

**Verified by**: `metrics::a_client_that_reconnects_tells_its_input_from_before` and `metrics::two_clients_on_one_pane_each_see_their_own_input`.

#### Scenario: A page reload
- **WHEN** a client reconnects (a new connection)
- **THEN** its new `seq_base` is greater than the old one, and the full frame it gets first carries the old connection's last seq, not one of the new connection's

#### Scenario: Two clients on one pane
- **WHEN** two clients send numbered input to one pane
- **THEN** their `seq_base`s differ and each sees both clients' seqs, in order, in `last_input_seq`

### Requirement: A client names its input
A client SHALL be able to name an input message (`KeyInput`, `MouseInput`, `Paste`, `RawInput`, `Scroll`, `Resize`) by `ClientMessage.seq = seq_base + k`, k = 1, 2, 3, ...; a seq with `seq_base <= seq < seq_base + 2^32` is that connection's, and `seq` 0 means not named. A client that numbers from 1, ignoring `seq_base`, SHALL still get its numbers echoed.

**Verified by**: `metrics::a_frame_names_the_input_it_shows_and_how_it_was_made`.

#### Scenario: Numbering from 1
- **WHEN** a client alone on a pane sends `RawInput` with seq 7, then a `Scroll` with seq 8
- **THEN** frames carry `last_input_seq` 7 and then 8

### Requirement: Frames name the input credited last
Every `Frame` SHALL carry `last_input_seq`: the seq of the input the pane credited last before building it, from whichever client, 0 while there is none; the latest, not the highest (another client's input can lower it). Input without a seq SHALL NOT change it.

**Verified by**: `metrics::a_frame_names_the_input_it_shows_and_how_it_was_made`.

#### Scenario: Before and after input
- **WHEN** a client subscribes to a new pane, then sends `RawInput` with seq 7
- **THEN** frames before it have `last_input_seq` 0, the frames after carry 7, and a line typed after it without a seq leaves 7

### Requirement: When input is credited
`KeyInput`, `MouseInput`, `Paste` and `RawInput` SHALL be credited once the pty has taken their last byte, in the order they arrived; `Scroll` and `Resize` when applied, at once, so they may be credited ahead of key input still waiting for the pty.

**Verified by**: `metrics::a_scroll_is_credited_while_key_input_waits_for_the_pty`.

#### Scenario: A scroll overtakes waiting key input
- **WHEN** 1 MiB of input waits for a pty nobody reads and a client sends a `Scroll` with a seq
- **THEN** a frame carries the scroll's seq while the key input is still not credited

### Requirement: Every credited input gets a frame
An input named by a seq SHALL be answered by a frame carrying its seq even when it changed nothing on the screen (a click the program ignores, an empty `RawInput`, a scroll that moves nothing). Input without a seq that changes nothing SHALL make no frame, and inputs credited between two frames SHALL show only the last of them.

**Verified by**: `metrics::input_that_changes_nothing_is_answered_with_an_empty_frame` and `metrics::a_frame_names_the_input_it_shows_and_how_it_was_made`.

#### Scenario: A scroll that moves nothing
- **WHEN** a client at the live screen sends `Scroll{to_bottom: true}` with seq 8
- **THEN** it gets a frame with `last_input_seq` 8 and no rows

#### Scenario: Unnamed input that changes nothing
- **WHEN** a client sends input that changes nothing without a seq
- **THEN** no frame comes within 300 ms

### Requirement: The frame for input that changed nothing
Such a frame SHALL have `full` false, no `changed` rows, no `styles`, no `title`, no `images`, `placements_changed` false and the pane's next `seq`, with `cols`, `rows`, `cursor`, `mouse_tracking`, `alt_screen`, the cell size and `viewport_top` as in the previous frame; its `timing` SHALL have `output_bytes` and `wait_us` 0.

**Verified by**: `metrics::input_that_changes_nothing_is_answered_with_an_empty_frame`.

#### Scenario: A click without mouse tracking
- **WHEN** a client sends a `MouseInput` with seq `seq_base + 1` to a shell that does not track the mouse
- **THEN** it gets a frame with `last_input_seq` `seq_base + 1`, no rows, `seq` one more than the last frame's, and that frame's cursor, size and `viewport_top`

### Requirement: Frames tell how the pane made them
Every frame SHALL carry `timing` (`FrameTiming`), in microseconds on the pane process's clock, saturating at u32::MAX: `wait_us`, `parse_us`, `build_us`, `encode_us`, `output_bytes`, `reads` and `input_us`, as below.

**Verified by**: `metrics::a_frame_names_the_input_it_shows_and_how_it_was_made`.

#### Scenario: A frame with output
- **WHEN** a command prints output
- **THEN** a frame carrying it has `timing` with `build_us` > 0, `output_bytes` > 0 and `reads` > 0

### Requirement: What the timing fields measure
`wait_us` SHALL be from the first pty output since the previous frame to the start of this frame's build (0 for a frame no output caused); `parse_us` feeding output into the screen model, summed; `build_us` building this frame; `encode_us` encoding and compressing the pane's previous frame (0 on its first); `output_bytes` and `reads` the pty output since the previous frame (output that changed nothing counts in the next frame) and the reads that brought it.

#### Scenario: A frame no output caused
- **WHEN** a frame answers a click that changed nothing
- **THEN** its `wait_us` and `output_bytes` are 0

### Requirement: input_us measures the pane's part of input latency
`input_us` SHALL be, for an input newly credited in this frame, the time from the pane process receiving it to the pty write that took its last byte returning (for `Scroll` and `Resize`: to it being applied), and 0 when no new input was credited.

**Verified by**: `metrics::the_last_input_and_input_still_waiting_survive_a_reload`.

#### Scenario: Input that waited for the pty
- **WHEN** input waits more than 1 s for the pty before it is taken
- **THEN** the frame that credits it has `input_us` over 1 000 000

### Requirement: Input latency survives a reload
A reload SHALL keep the pane's `last_input_seq` and its input still waiting for the pty; that input SHALL be credited after the reload, with `input_us` counted from when the old pane process received it.

**Verified by**: `metrics::the_last_input_and_input_still_waiting_survive_a_reload`.

#### Scenario: Waiting input across a reload
- **WHEN** 1 MiB of `RawInput` with seq `seq_base + 2` waits for a pty whose program sleeps through a reload, after input `seq_base + 1` was credited
- **THEN** after the reload the full frame carries `seq_base + 1`, a later frame carries `seq_base + 2` with `input_us` over 1 s, and a new connection's `seq_base` is greater than the old one's

### Requirement: A lagging client skips to the latest screen
For a client the server judges to lag (see `client-liveness`) the frames of a pane SHALL be skipped, each counted in `tilrd_pane_frames_skipped_total`, until its queue is written out; it SHALL then get a full frame of the screen as it is (asked of the pane, so its other viewers get it too). It SHALL NOT be dropped for lagging, and other clients SHALL NOT be slowed.

**Verified by**: `metrics::a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo` and itests `UC-049` (`uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others`).

#### Scenario: A slow client under heavy output
- **WHEN** one client on a 10 KB/s link and one on a fast link watch a pane printing heavily (`uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others`)
- **THEN** frames are skipped for the slow one, it ends on the latest screen, and the fast one is not slowed

### Requirement: A lagging client still gets its echo
The full frame a lagging client gets after skipping SHALL carry in `last_input_seq` the client's own latest seq when a skipped frame echoed it, so every numbered input of a client gets a frame that echoes it.

**Verified by**: `metrics::a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo`.

#### Scenario: Echo in a skipped frame
- **WHEN** a client that does not read sends numbered input to a pane printing a stream of output, then reads again
- **THEN** it gets a full frame and the end of the output, that frame carries its own input's seq, and it stays connected

### Requirement: The metrics endpoint
An HTTP request on the TCP port whose request line starts with `GET /metrics` followed by a space or `?` SHALL be answered with the metrics instead of a WebSocket; a request line arriving in pieces SHALL be waited for up to 1 s. The answer SHALL be `200` with `Content-Type: text/plain; version=0.0.4; charset=utf-8` (Prometheus text format) and `Connection: close`.

**Verified by**: `metrics::metrics_ask_for_the_access_token`, `metrics::a_metrics_request_may_come_in_pieces`.

#### Scenario: A request in pieces
- **WHEN** a scraper sends `GET /met`, then `rics?token=<token> HTTP/1.1\r\nHost: ...\r`, then `\n\r`, then `\n`
- **THEN** it gets `200` with `# TYPE tilrd_panes gauge` in the body

#### Scenario: The protocol on the same port
- **WHEN** a client then opens a WebSocket on the same port
- **THEN** it talks the protocol as before (`ListPanes` is answered)

### Requirement: Limits of a metrics request
The request head SHALL have to arrive within 10 s and fit in 16 KiB, else the answer SHALL be `400`; a response not written within 5 s SHALL be dropped; if the server's event loop does not hand over its counters within 5 s the answer SHALL be `503`. The Unix socket SHALL NOT serve metrics.

**Verified by**: unit test `metrics::tests::request_head_is_parsed` (crates/tilrd/src/metrics.rs).

#### Scenario: A head that never ends
- **WHEN** a client sends `GET /metrics HTTP/1.1` and no blank line for 10 s
- **THEN** it gets `400` and the connection is closed

### Requirement: Metrics ask for the access token
The endpoint SHALL ask for the access token as `/socks` does: `Authorization: Bearer <token>` or `?token=<token>`. Without it, or with a wrong one, the answer SHALL be `401` with `WWW-Authenticate: Bearer`; a request with an `Origin` that is not allowed SHALL get `403` whatever its token.

**Verified by**: `metrics::metrics_ask_for_the_access_token`.

#### Scenario: No or wrong token
- **WHEN** `/metrics` is asked without a token, with `?token=wrong`, or with `Authorization: Bearer wrong`
- **THEN** each gets `401`

#### Scenario: Disallowed origin
- **WHEN** `/metrics` is asked with the right token and `Origin: https://evil.example`
- **THEN** it gets `403`

### Requirement: Without a token, metrics are for this machine only
On a server without a token (`--no-auth`) the endpoint SHALL ask for none but SHALL answer `403` unless `Host` names loopback (`localhost`, `127.0.0.1`, `[::1]`, with or without a port) or the address listened on, and `403` to `Sec-Fetch-Site: cross-site` or no `Host`. With a token, the token alone SHALL decide.

**Verified by**: `metrics::metrics_need_no_token_on_a_server_without_one`, `metrics::without_a_token_metrics_are_for_this_machine_only`.

#### Scenario: DNS rebinding
- **WHEN** a `--no-auth` server gets `/metrics` with `Host: evil.example:<port>` or `Host: localhost.evil.example:<port>`
- **THEN** it answers `403`

#### Scenario: Loopback names
- **WHEN** a `--no-auth` server gets `/metrics` with `Host` `localhost`, `localhost:<port>`, `[::1]:<port>` or `127.0.0.1`, or with `Sec-Fetch-Site` `same-origin` or `none`
- **THEN** it answers `200`

#### Scenario: A token decides on a server with one
- **WHEN** a server with a token gets `/metrics?token=<token>` with `Host: evil.example` and `Sec-Fetch-Site: cross-site`
- **THEN** it answers `200`

### Requirement: The metric families
Every family name SHALL start with `tilrd_`. There SHALL be `# HELP` and `# TYPE` for each family, even without samples: gauges `tilrd_info{version,server_id}` (always 1), `tilrd_clients`, `tilrd_panes`; the per-pane counters; `tilrd_client_drops_total{reason}`; histograms `tilrd_route_seconds`, `tilrd_route_input_seconds`, `tilrd_supervisor_loop_lag_seconds`; `tilrd_process_cpu_seconds_total`, `tilrd_process_memory_bytes`.

**Verified by**: unit test `metrics::tests::exposition_lists_every_family_with_help_and_type` (crates/tilrd/src/metrics.rs), `metrics::metrics_count_frames_input_and_routing`.

#### Scenario: A fresh server
- **WHEN** `/metrics` is scraped on a server with no panes
- **THEN** `tilrd_panes` is 0 and a `tilrd_info{version="...",server_id="..."} 1` line is present

#### Scenario: One prefix
- **WHEN** `/metrics` is scraped on a server with one pane after some traffic
- **THEN** every `# TYPE` line and every sample names a family starting with `tilrd_`, and none starts with `tilr_`

### Requirement: Per-pane frame counters
For each running pane (label `pane` = its id) the server SHALL count `tilrd_pane_frames_total` (frames the pane produced), `tilrd_pane_frame_bytes_total` (their bytes as routed, once per frame whatever the number of clients), `tilrd_pane_frames_sent_total` (one per frame and client it was handed to) and `tilrd_pane_frames_skipped_total` (one per frame and lagging client it was not sent to).

**Verified by**: `metrics::metrics_count_frames_input_and_routing` and itests `UC-049` (`uc_049.py::a_slow_client_under_heavy_output_gets_the_latest_screens_without_slowing_the_others`).

#### Scenario: After typing a command
- **WHEN** a client typed a command into a pane and saw its output
- **THEN** that pane's frames, frame bytes and frames sent are each at least 1

### Requirement: Per-pane input counters
`tilrd_pane_input_total{pane}` and `tilrd_pane_input_bytes_total{pane}` SHALL count the key, mouse, paste, raw and scroll packets passed to the pane and their bytes (`Resize` is not counted). All per-pane counters SHALL start from zero when the supervisor starts, a reload included, and a pane's series SHALL go when the pane does.

**Verified by**: `metrics::metrics_count_frames_input_and_routing`.

#### Scenario: Typed input
- **WHEN** a client typed a command into a pane
- **THEN** that pane's input and input bytes are each at least 1

### Requirement: Clients let go, by reason
`tilrd_client_drops_total{reason}` SHALL count the clients the server let go, with a sample from 0 for each of `superseded` (by a newer connection of the same client), `too_slow` (its queue filled up), `silent` (nothing heard for the client timeout), `no_pong` (it sent, but answered no ping for the client timeout) and `no_hello` (no `Hello` within 10 s of its WebSocket handshake). A client that disconnects on its own, or that a handshake check turns away with an `Error`, SHALL NOT be counted.

**Verified by**: `metrics::metrics_count_the_clients_let_go_and_why`, `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello` and itests `UC-052` (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`).

#### Scenario: Superseded
- **WHEN** a second connection says hello with the `client_id` and token of an open one
- **THEN** `tilrd_client_drops_total{reason="superseded"}` goes from 0 to 1 and `too_slow` stays 0

#### Scenario: Superseded over the network
- **WHEN** a client's connection is superseded (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`)
- **THEN** `tilrd_client_drops_total{reason="superseded"}` is 1

#### Scenario: No Hello
- **WHEN** a client completes the WebSocket handshake, sends no `Hello`, and is closed with 4002 `timeout` about 10 s later
- **THEN** `tilrd_client_drops_total{reason="no_hello"}` goes from 0 to 1 and the other reasons stay 0

#### Scenario: A wrong token is not a drop
- **WHEN** a TCP client says `Hello` with a wrong token and is answered with an `Error`
- **THEN** no `tilrd_client_drops_total` sample changes

### Requirement: Routing and event-loop histograms
`tilrd_route_seconds` SHALL observe a pane's packet from the supervisor reading it off the pane process's pipe to the WebSocket write to the client returning; `tilrd_route_input_seconds` a client's input from the supervisor reading it off the WebSocket to the write to the pane process returning; `tilrd_supervisor_loop_lag_seconds` how late the event loop gets to a once-a-second tick.

**Verified by**: `metrics::metrics_count_frames_input_and_routing`.

#### Scenario: After some traffic
- **WHEN** a client typed into a pane and saw output, and a second has passed
- **THEN** `tilrd_route_seconds_count`, `tilrd_route_input_seconds_count` and `tilrd_supervisor_loop_lag_seconds_count` are each at least 1

### Requirement: Histogram buckets
The histograms SHALL have the buckets 0.00005, 0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1 and +Inf seconds, cumulative, with `_sum` and `_count`.

**Verified by**: unit test `metrics::tests::histogram_buckets_are_cumulative` (crates/tilrd/src/metrics.rs) and `metrics::metrics_count_frames_input_and_routing`.

#### Scenario: The +Inf bucket
- **WHEN** packets were routed
- **THEN** `tilrd_route_seconds_bucket{le="+Inf"}` equals `tilrd_route_seconds_count` and is at least 1

### Requirement: CPU and memory of the server's processes
`tilrd_process_cpu_seconds_total` (user plus system CPU) and `tilrd_process_memory_bytes` (resident set on Linux, physical footprint on macOS) SHALL be read when scraped, for the supervisor (`role="supervisor"`, `pid`) and each pane process (`role="pane"`, `pid`, `pane`), not the programs in the panes; a process whose usage cannot be read SHALL have no sample.

**Verified by**: `metrics::metrics_count_frames_input_and_routing` and unit test `metrics::tests::this_process_has_usage`.

#### Scenario: Supervisor and pane
- **WHEN** one pane runs and `/metrics` is scraped
- **THEN** `tilrd_process_memory_bytes{role="supervisor",pid="<server pid>"}` and `tilrd_process_memory_bytes{role="pane",pid="<pane pid>",pane="<id>"}` are at least 1

### Requirement: Latency is measurable over a bad network
A client SHALL be able to measure input latency on its own clock, from sending a numbered input to the first frame carrying its seq (or a later one of its own), over a lossy, slow network without the session breaking.

**Verified by**: itests `UC-051` (`uc_051.py::a_shell_stays_usable_on_a_3g_network_and_input_latency_is_recorded`).

#### Scenario: 3G profile
- **WHEN** a client types key by key and runs 30 commands over a link with 150 ms ± 20 ms delay, 1% loss and 10% reordering each way
- **THEN** every key is echoed, key echo p90 is under 2 s, input latency p99 under 5 s, and the session never disconnects

## Notes

- Requirements without a test: "What the timing fields measure" (only `build_us`, `output_bytes`, `reads`, and `wait_us` 0 on an empty frame are asserted). Parts without a test: the 400 for a head over 16 KiB or 10 s, the 5 s response timeout, the 503 after 5 s, the counters resetting on a reload, the `silent` and `no_pong` reasons through `/metrics` (the drops themselves are tested in `client-liveness`).
- Unit tests are cited as `module::tests::name`; `metrics::tests::*` are in crates/tilrd/src/metrics.rs. Also relevant: `metrics::tests::stat_times_are_counted_from_the_name` (Linux CPU time parsing) and `supervisor::tests::metrics_without_a_token_are_for_this_machine_only` (crates/tilrd/src/supervisor.rs).
- The "head that never ends" scenario of "Limits of a metrics request" is not tested end to end; `metrics::tests::request_head_is_parsed` checks only the parsing of a head.
