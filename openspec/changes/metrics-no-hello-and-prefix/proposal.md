# Proposal: count connections closed for no Hello, and one prefix (`tilr_`) for every metric

## Decision (2026-10-18)

Owner: option (b), "везде tilr_": every metric family uses the prefix `tilr_`. The three gauges `tilrd_info`, `tilrd_clients`, `tilrd_panes` are renamed back to `tilr_info`, `tilr_clients`, `tilr_panes`; the twelve `tilr_*` families stay. Rejected: (a) `tilrd_` (twelve renames). Prometheus naming asks for one application prefix per exporter (`node_`, `prometheus_`); `tilr_` is the product's prefix. Recorded in Itsaplan TILR-109.

## Why

Itsaplan TILR-109 (phase 5, tech debt). Checked at 048b0e2:

- Commit f32643d (archived change `2026-10-18-hello-deadline`) closes a main-protocol connection that has not said `Hello` 10 s after its handshake (`close_ungreeted`, `crates/tilrd/src/supervisor.rs:2542-2554`, `Gone::NoHello`, `supervisor.rs:257`). `drop_client` logs it and closes with 4002 `timeout` (`supervisor.rs:2625-2633`) but counts only `Gone::Dropped` (`supervisor.rs:2634-2635`, `self.metrics.count_drop(drop)`), and `ClientDrop` has no such reason (`crates/tilrd/src/metrics.rs:78-104`: `superseded`, `too_slow`, `silent`, `no_pong`). So `tilr_client_drops_total` does not show a scanner or a stuck client holding connections open, which is what the deadline was added against.
- The families use two prefixes (`crates/tilrd/src/metrics.rs:154-280`; README "Metrics", `README.md:841-857`): `tilrd_info`, `tilrd_clients`, `tilrd_panes`, and `tilr_` for the other twelve. Commit e6885b0 ("The server is tilrd: ...") renamed `tilr_server_info`, `tilr_server_clients` and `tilr_server_panes` to `tilrd_info`, `tilrd_clients`, `tilrd_panes` and left the `tilr_` families as they were; rationale for leaving them not recorded.
- No release has been cut yet (no tag in the repository; `openspec/ROADMAP.md` "Release gate": the first archive release waits for phase 1 and more), so a rename now breaks only dashboards built on dev builds.

## What Changes

- A connection closed for saying no `Hello` within 10 s counts in the client drops metric with `reason="no_hello"`, which has a sample from 0 like the other reasons. Connections the handshake checks turn away (wrong token, protocol version, origin) and clients that disconnect on their own stay uncounted.
- **BREAKING** (dashboards, alerts, scrapers): every family is `tilr_`. Renamed: `tilrd_info` → `tilr_info`, `tilrd_clients` → `tilr_clients`, `tilrd_panes` → `tilr_panes`. The twelve `tilr_*` families, labels and values are unchanged. Migration: replace `tilrd_info|tilrd_clients|tilrd_panes` with `tilr_info|tilr_clients|tilr_panes` in queries, recording rules and alerts (the rename list goes into README "Metrics"); no family is emitted under both names.

## Capabilities

### Modified Capabilities
- `metrics`: see `specs/metrics/spec.md`.

## Non-goals

- Counting connections closed by the 10 s WebSocket-handshake timeout (before a WebSocket exists; no client in the supervisor) or raw SOCKS and `/metrics` connections.
- New metrics or labels; changing the label `reason` values that exist.
- A transition period with both names (no release has shipped the old ones).
- Protocol: no change to tilr.proto, close codes or `PROTOCOL_VERSION`.

## Impact

`crates/tilrd/src/metrics.rs` (the three gauge names, `ClientDrop::NoHello`, unit tests), `crates/tilrd/src/supervisor.rs` (`drop_client` counts `no_hello`), `crates/tilrd/tests/metrics.rs` (reads `tilrd_panes`, `tilrd_clients`, `tilrd_info`), `openspec/specs/metrics/spec.md` (via the delta), README "Metrics", USE_CASES.md. The itests (`uc_043.py`, `uc_049.py`, `uc_052.py`) read only `tilr_pane_frames_skipped_total`, `tilr_pane_frames_total` and `tilr_client_drops_total`: no itest reads a `tilrd_*` name, so none changes. Metrics restart from zero on a reload as today; nothing crosses a reload and the state formats are unchanged. An update from the previous build renames the three gauges at the first scrape after the reload into it: the old series end there. **BREAKING** for anything that queries `tilrd_info`, `tilrd_clients` or `tilrd_panes`; clients of the protocol are not affected.
