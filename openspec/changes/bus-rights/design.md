# Design: Biscuit rights on the bus

Shared design: `openspec/design/unified-bus.md` Part R (R1-R9), §B2, §W2 (origin as a fact).
This change's part follows.

## Decisions

- **One swap point.** The Biscuit authorizer implements the `Authorizer` trait of
  `bus-broker-core`'s contract; `default_authorizer` returns it; no caller changes. Instances
  get a token minted from their `Rights` at each start, through `hello_ack_credential`.
- **Pane facts are minted, not checked, here.** `pane = [...]` rights become the facts of R3 in
  the token; the core's second authorization (T7) is `bus-pane-rights`'s.
- **Commands later.** Tests mint with `bus.token`, revoke with `bus.revoke` and rotate with
  `bus.rotate_key` directly; `tilr bus token|revoke|rotate-key` (named in the scenarios) are
  `bus-tilr-client`'s.
- **Benchmark.** `benches/authorize.rs` (criterion) is recorded in the PR; a cached decision
  under 1 µs on CI hardware is a note, not a gate.

## Reload and reconnect

Until `bus-reload`, a reload restarts instances (`bus-instances`' interim) and they get new
tokens; the old ones are revoked when their process ends. Outside participants reconnect with
their token. `bus-reload` hands over revocation ids and renewal times (B13).

## Backpressure

Authorization cost is bounded by R7 (1 ms run limit, 1000 facts, 100 iterations, token ≤ 16 KiB
and ≤ 8 blocks, parsed-token LRU 1024, decision cache TTL 1 s).
