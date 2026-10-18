## MODIFIED Requirements

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

**Verified by**: `metrics::metrics_count_the_clients_let_go_and_why`, `metrics::a_connection_that_never_says_hello_is_counted_as_no_hello` and itests `UC-052` (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`).

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
