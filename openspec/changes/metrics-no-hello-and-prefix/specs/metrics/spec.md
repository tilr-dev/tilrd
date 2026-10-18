## MODIFIED Requirements

### Requirement: The metrics endpoint
An HTTP request on the TCP port whose request line starts with `GET /metrics` followed by a space or `?` SHALL be answered with the metrics instead of a WebSocket; a request line arriving in pieces SHALL be waited for up to 1 s. The answer SHALL be `200` with `Content-Type: text/plain; version=0.0.4; charset=utf-8` (Prometheus text format) and `Connection: close`.

**Verified by**: `metrics::metrics_ask_for_the_access_token`, `metrics::a_metrics_request_may_come_in_pieces`.

#### Scenario: A request in pieces
- **WHEN** a scraper sends `GET /met`, then `rics?token=<token> HTTP/1.1\r\nHost: ...\r`, then `\n\r`, then `\n`
- **THEN** it gets `200` with `# TYPE tilr_panes gauge` in the body

#### Scenario: The protocol on the same port
- **WHEN** a client then opens a WebSocket on the same port
- **THEN** it talks the protocol as before (`ListPanes` is answered)

### Requirement: The metric families
Every family name SHALL start with `tilr_`. There SHALL be `# HELP` and `# TYPE` for each family, even without samples: gauges `tilr_info{version,server_id}` (always 1), `tilr_clients`, `tilr_panes`; the per-pane counters; `tilr_client_drops_total{reason}`; histograms `tilr_route_seconds`, `tilr_route_input_seconds`, `tilr_supervisor_loop_lag_seconds`; `tilr_process_cpu_seconds_total`, `tilr_process_memory_bytes`.

**Verified by**: unit test `metrics::tests::exposition_lists_every_family_with_help_and_type` (crates/tilrd/src/metrics.rs), `metrics::metrics_count_frames_input_and_routing`.

#### Scenario: A fresh server
- **WHEN** `/metrics` is scraped on a server with no panes
- **THEN** `tilr_panes` is 0 and a `tilr_info{version="...",server_id="..."} 1` line is present

#### Scenario: One prefix
- **WHEN** `/metrics` is scraped on a server with one pane after some traffic
- **THEN** every `# TYPE` line and every sample names a family starting with `tilr_`, and none starts with `tilrd_`

### Requirement: Clients let go, by reason
`tilr_client_drops_total{reason}` SHALL count the clients the server let go, with a sample from 0 for each of `superseded` (by a newer connection of the same client), `too_slow` (its queue filled up), `silent` (nothing heard for the client timeout), `no_pong` (it sent, but answered no ping for the client timeout) and `no_hello` (no `Hello` within 10 s of its WebSocket handshake). A client that disconnects on its own, or that a handshake check turns away with an `Error`, SHALL NOT be counted.

**Verified by**: `metrics::metrics_count_the_clients_let_go_and_why`, `metrics::a_connection_that_never_says_hello_is_counted_as_no_hello` and itests `UC-052` (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`).

#### Scenario: Superseded
- **WHEN** a second connection says hello with the `client_id` and token of an open one
- **THEN** `tilr_client_drops_total{reason="superseded"}` goes from 0 to 1 and `too_slow` stays 0

#### Scenario: Superseded over the network
- **WHEN** a client's connection is superseded (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`)
- **THEN** `tilr_client_drops_total{reason="superseded"}` is 1

#### Scenario: No Hello
- **WHEN** a client completes the WebSocket handshake, sends no `Hello`, and is closed with 4002 `timeout` about 10 s later
- **THEN** `tilr_client_drops_total{reason="no_hello"}` goes from 0 to 1 and the other reasons stay 0

#### Scenario: A wrong token is not a drop
- **WHEN** a TCP client says `Hello` with a wrong token and is answered with an `Error`
- **THEN** no `tilr_client_drops_total` sample changes
