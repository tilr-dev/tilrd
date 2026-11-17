# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` before the split).

## 1. Broker skeleton and extension points

- [x] 1.1 [2.1, skeleton] `crates/tilrd/src/bus/`: the module tree and stub files of the design, `identity.rs`, `authz.rs` with the stub authorizer, `core/mod.rs` with the native participant `tilrd` and its registry, `BrokerHandle`, `SupervisorHandle` with `CoreCmd::With` and `BusObserver`; verify `cargo build --workspace --locked` and a unit test of the stub authorizer's patterns (`authz::tests::the_stub_allows_what_the_rights_name`)
- [x] 1.2 [2.1, skeleton] `broker.rs`, first `events.rs` and `streams.rs`: routing, `from`, reply check, `Select`/`Activate`, per-pair order, reserved namespaces, plain streams with `Cancel`, events with NATS patterns (B5, B6, B7 without credits); verify `bus::from_is_always_the_callers_identity`, `bus::calls_go_round_robin_over_the_providers`, `bus::last_active_goes_to_the_provider_that_activated_last`, `bus::reserved_namespaces_cannot_be_provided`, `bus::calls_between_two_participants_keep_their_order`, `bus::a_stream_is_a_call_answered_in_chunks`, `bus::events_reach_the_subscribers_connected_now`, unit test `events::tests::patterns_match_as_nats_subjects`; USE_CASES.md entries (own section)

## 2. WebSocket transport beside tilr.v1

- [x] 2.1 [2.3] WebSocket transport of the bus on the TCP port and the Unix socket under the subprotocol `tilr.bus.v1`, Hello/HelloAck, `ConnectionError`, owner by peer uid or access token, liveness and superseding kept, `bus.ping`; `tilr.v1` still served without the subprotocol (O2); verify `bus::hello_comes_first_and_names_a_version`, `bus::the_first_thing_a_participant_hears_is_its_hello_ack`, `bus::unknown_envelopes_are_ignored_and_bad_ones_reported`, and every existing test green unchanged
- [x] 2.2 Test harness `crates/tilrd/tests/common/bus.rs` and the lanes' stub files (design "Test harness"); README "The bus" (new section) with the transports, Hello and empty subsections for streams, instances, rights and Tilr Web; verify the tests of 1.2 and 2.1 use it and README review

## 3. Checks and archive

- [x] 3.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [x] 3.2 [12.2] Update this change's row in `openspec/ROADMAP.md` and archive the change; verify `openspec validate --all --strict`
