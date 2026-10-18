# Tasks

## 1. The tilr-sync crate

- [ ] 1.1 Choose iroh or quinn (design open question) and record it in design.md; verify `cargo deny check` with the new dependencies
- [ ] 1.2 Index, block transfer, conflicts and name rules in `crates/tilr-sync` with unit tests for NFC/NFD, `A.txt`/`a.txt`, deletes and renames not resurrected, conflict copies; verify `cargo test -p tilr-sync`

## 2. Embedding in tilrd

- [ ] 2.1 `[sync]` config, endpoint and `sync` feature; verify with `sync::sync_can_be_turned_off`; README configuration section
- [ ] 2.2 `SyncPair` / `SyncPaired` / `SyncUnpair` in `tilr.proto` and pairing (design D3); verify with `sync::an_unpaired_peer_is_refused` and `sync::a_paired_peer_sees_the_roots`; README "Protocol"
- [ ] 2.3 UDP socket across a reload (design D4, supervisor-state bump, `state-formats`); verify with `reload::the_sync_endpoint_survives_a_reload`
- [ ] 2.4 itests group S cases (first sync, ≤10 ms push in a LAN, catch-up after a cut, partial files) with spec tags, USE_CASES.md entries; verify `./itests/dt` group S and `./itests/dt coverage`

## 3. Checks

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
