# Tasks

## 0. Decision

- [ ] 0.1 The transport decision (iroh vs quinn) is made, then reread D2-D4 against it; the default is the prepared plan (off, like SOCKS) unless the owner says otherwise; verify `openspec validate port-forwarding --strict`

## 1. Protocol and server

- [ ] 1.1 `ForwardListen`, `ForwardListening`, `ForwardStop`, `StreamOpen.forward_id`, kind `tcp-forward`, feature `"forward"` in `tilr.proto`; README "Protocol"; verify `cargo test -p tilr-proto`
- [ ] 1.2 `forward = true|false` in the configuration and `--forward` / `--no-forward`; feature listed accordingly; verify with unit test `config::tests::forwarding_follows_the_flag_and_the_file` and `forward::a_client_asks_the_server_to_listen_for_it`
- [ ] 1.3 `crates/tilrd/src/forward.rs`: loopback listeners per client, 16 per client, streams sharing the agent's stream table, limits and backpressure (D2, D3); verify with `forward::bytes_go_both_ways_through_a_forward` and `forward::a_forward_ends_with_its_client`
- [ ] 1.4 Forwards end with a reload; verify with `reload::forwards_end_with_a_reload_and_tilr_asks_again`; USE_CASES.md entries

## 2. tilr forward

- [ ] 2.1 `tilr forward -R` (D5) with reconnect; verify with `client::tilr_forward_carries_connections_both_ways` (part -R)
- [ ] 2.2 `tilr forward -L` over `/socks` with the token; refusal without SOCKS or over the Unix socket; verify with `client::tilr_forward_carries_connections_both_ways` (part -L)
- [ ] 2.3 README section "Port forwarding" (both forms, the switch, loopback only, what a reload does); verify by review against this proposal

## 3. itests

- [ ] 3.1 UC-07E in itests/USE_CASES.md and `itests/scenarios/uc_07e.py::psql_on_the_server_reaches_postgres_on_the_laptop_through_tilr_forward` (hosts `laptop` with a service, server runs `psql`/`curl` to the forwarded port), tagged `port-forwarding/a-connection-to-a-forward-opens-a-stream-to-its-client`; verify `./itests/dt UC-07E` green

## 4. Checks and archive

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 4.2 `./itests/dt UC-073 UC-074 UC-07E` green, then `./itests/dt coverage` exits 0
- [ ] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
