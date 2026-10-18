# Design

## Context

At 048b0e2 `crates/tilrd/src/metrics.rs:154-280` renders three `tilrd_` gauges (to become `tilr_`) and twelve `tilr_` families; the drop reasons are `ClientDrop` (`metrics.rs:78-104`), counted in `Supervisor::drop_client` for `Gone::Dropped` only (`crates/tilrd/src/supervisor.rs:2634-2635`); `Gone::NoHello` (`supervisor.rs:2625-2633`) and `Gone::Refused` (`supervisor.rs:2624`) are not counted.

## Decisions

### D1. The prefix is `tilr_`
Owner decision (2026-10-18, "везде tilr_"). Prometheus practice is one application prefix per exporter; `tilr_` is the product's name, and twelve families already carry it, so only the three gauges `tilrd_info`, `tilrd_clients`, `tilrd_panes` are renamed (back to `tilr_info`, `tilr_clients`, `tilr_panes`, as before e6885b0 minus the `_server`).
- Rejected: `tilrd_` for all (twelve renames). It follows the binary's name, which e6885b0 gave to the three gauges, but costs four times the renames for the same one-prefix result; the owner chose the product prefix.

### D2. No double emission
The old names are not emitted alongside the new ones for a transition: nothing released carries them, and a doubled exposition doubles every series a scraper stores.

### D3. `no_hello` is a `ClientDrop`
`ClientDrop` gains `NoHello` (label `no_hello`, last in `ALL`, so the existing samples keep their order); `close_ungreeted` drops with `Gone::Dropped(ClientDrop::NoHello)` and the `NoHello` arm of `Gone` goes, its log line (`client sent no Hello in 10s, dropping`) and close (4002 `timeout`) moved into the `ClientDrop::NoHello` arm. `Gone::Refused` stays uncounted: a refused handshake (bad token, version, origin) is the client's mistake, answered with an `Error`, not the server letting a client go.

## Reload and reconnect

The counters live in the supervisor (`metrics::Shared`) and restart from zero after a reload, as today; nothing is handed over. A connection not yet greeted when a reload closes every connection is not counted (it is closed by the reload, not by the deadline). Clients reconnecting are not affected.

## Backpressure

Not applicable: a counter per reason, no client data.
