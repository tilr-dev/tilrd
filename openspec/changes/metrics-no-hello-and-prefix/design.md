# Design

## Context

At 048b0e2 `crates/tilrd/src/metrics.rs:154-280` renders three `tilrd_` gauges and twelve `tilr_` families; the drop reasons are `ClientDrop` (`metrics.rs:78-104`), counted in `Supervisor::drop_client` for `Gone::Dropped` only (`crates/tilrd/src/supervisor.rs:2634-2635`); `Gone::NoHello` (`supervisor.rs:2625-2633`) and `Gone::Refused` (`supervisor.rs:2624`) are not counted.

## Decisions

### D1. The prefix is `tilrd_`
The process that serves the metrics is `tilrd`, and e6885b0 named "the metrics" after it when it renamed the binary; Prometheus practice is one prefix per exporter, the application's name. `tilr_` is the product and also the client binary (`tilr`), which serves no metrics.
- Rejected: `tilr_` for all (three renames instead of twelve). Fewer renames, but against the recorded intent of e6885b0 and ambiguous with the `tilr` client. With no release cut yet the number of renames costs little.

### D2. No double emission
The old names are not emitted alongside the new ones for a transition: nothing released carries them, and a doubled exposition doubles every series a scraper stores.

### D3. `no_hello` is a `ClientDrop`
`ClientDrop` gains `NoHello` (label `no_hello`, last in `ALL`, so the existing samples keep their order); `close_ungreeted` drops with `Gone::Dropped(ClientDrop::NoHello)` and the `NoHello` arm of `Gone` goes, its log line (`client sent no Hello in 10s, dropping`) and close (4002 `timeout`) moved into the `ClientDrop::NoHello` arm. `Gone::Refused` stays uncounted: a refused handshake (bad token, version, origin) is the client's mistake, answered with an `Error`, not the server letting a client go.

## Reload and reconnect

The counters live in the supervisor (`metrics::Shared`) and restart from zero after a reload, as today; nothing is handed over. A connection not yet greeted when a reload closes every connection is not counted (it is closed by the reload, not by the deadline). Clients reconnecting are not affected.

## Backpressure

Not applicable: a counter per reason, no client data.
