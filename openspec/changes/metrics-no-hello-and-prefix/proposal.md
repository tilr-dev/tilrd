# Proposal: count connections closed for no Hello, and one prefix for every metric

Блокер: which prefix every metric keeps — (a) `tilrd_` (renames the 12 `tilr_*` families; recommended, written below) or (b) `tilr_` (renames back the 3 `tilrd_*` gauges)? Asked in Itsaplan TILR-109.

## Why

Itsaplan TILR-109 (phase 5, tech debt). Checked at 048b0e2:

- Commit f32643d (archived change `2026-10-18-hello-deadline`) closes a main-protocol connection that has not said `Hello` 10 s after its handshake (`close_ungreeted`, `crates/tilrd/src/supervisor.rs:2542-2554`, `Gone::NoHello`, `supervisor.rs:257`). `drop_client` logs it and closes with 4002 `timeout` (`supervisor.rs:2625-2633`) but counts only `Gone::Dropped` (`supervisor.rs:2634-2635`, `self.metrics.count_drop(drop)`), and `ClientDrop` has no such reason (`crates/tilrd/src/metrics.rs:78-104`: `superseded`, `too_slow`, `silent`, `no_pong`). So `tilr_client_drops_total` does not show a scanner or a stuck client holding connections open, which is what the deadline was added against.
- The families use two prefixes (`crates/tilrd/src/metrics.rs:154-280`; README "Metrics", `README.md:841-857`): `tilrd_info`, `tilrd_clients`, `tilrd_panes`, and `tilr_` for the other twelve. Commit e6885b0 ("The server is tilrd: the crate, the directory, the binary, clap and the metrics are named tilrd") renamed `tilr_server_info`, `tilr_server_clients` and `tilr_server_panes` to `tilrd_info`, `tilrd_clients`, `tilrd_panes` and left the `tilr_` families as they were; rationale for leaving them not recorded.
- No release has been cut yet (no tag in the repository; `openspec/ROADMAP.md` "Release gate": the first archive release waits for phase 1 and more), so a rename now breaks only dashboards built on dev builds.

## What Changes

- A connection closed for saying no `Hello` within 10 s counts in the client drops metric with `reason="no_hello"`, which has a sample from 0 like the other reasons. Connections the handshake checks turn away (wrong token, protocol version, origin) and clients that disconnect on their own stay uncounted.
- **BREAKING** (dashboards, alerts, scrapers): every family is `tilrd_`. Renamed: `tilr_pane_frames_total` → `tilrd_pane_frames_total`, `tilr_pane_frame_bytes_total` → `tilrd_pane_frame_bytes_total`, `tilr_pane_frames_sent_total` → `tilrd_pane_frames_sent_total`, `tilr_pane_frames_skipped_total` → `tilrd_pane_frames_skipped_total`, `tilr_pane_input_total` → `tilrd_pane_input_total`, `tilr_pane_input_bytes_total` → `tilrd_pane_input_bytes_total`, `tilr_client_drops_total` → `tilrd_client_drops_total`, `tilr_route_seconds` → `tilrd_route_seconds`, `tilr_route_input_seconds` → `tilrd_route_input_seconds`, `tilr_supervisor_loop_lag_seconds` → `tilrd_supervisor_loop_lag_seconds`, `tilr_process_cpu_seconds_total` → `tilrd_process_cpu_seconds_total`, `tilr_process_memory_bytes` → `tilrd_process_memory_bytes` (histograms with their `_bucket`, `_sum`, `_count`). Labels and values are unchanged. Migration: replace `tilr_` with `tilrd_` in queries, recording rules and alerts (the rename list goes into README "Metrics"); no family is emitted under both names.

## Capabilities

### Modified Capabilities
- `metrics`: see `specs/metrics/spec.md`.

## Non-goals

- Counting connections closed by the 10 s WebSocket-handshake timeout (before a WebSocket exists; no client in the supervisor) or raw SOCKS and `/metrics` connections.
- New metrics or labels; changing the label `reason` values that exist.
- A transition period with both names (no release has shipped the old ones).
- Protocol: no change to tilr.proto, close codes or `PROTOCOL_VERSION`.

## Impact

`crates/tilrd/src/metrics.rs` (names, `ClientDrop::NoHello`, unit tests), `crates/tilrd/src/supervisor.rs` (`drop_client` counts `no_hello`), `crates/tilrd/tests/metrics.rs`, itests `uc_043.py`, `uc_049.py`, `uc_052.py` (they read `tilr_pane_frames_skipped_total`, `tilr_pane_frames_total`, `tilr_client_drops_total`), README "Metrics", USE_CASES.md. Metrics restart from zero on a reload as today; nothing crosses a reload and the state formats are unchanged. An update from the previous build renames the series at the first scrape after the reload into it: the old series end there. **BREAKING** for anything that queries the old names (above); clients of the protocol are not affected.
