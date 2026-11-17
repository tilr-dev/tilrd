# Tasks

## 1. Crate tilr-bus

- [ ] 1.1 `crates/tilr-bus`: `proto/bus.proto` (D3), framing with the 16 MiB limit both ways, error codes, async client (`connect_from_env` for fd and socket, `register`, `call`, `serve`, `Cancel` handling); workspace member; verify unit tests `codec::tests::a_frame_over_the_limit_is_refused`, `codec::tests::unknown_envelope_variants_are_ignored`; `cargo deny check`
- [ ] 1.2 Test runtime `crates/tilr-bus/examples/test_runtime.rs` (`test.echo`, `test.whoami`, `test.from`, `test.env`, `test.sleep`, `test.nested`, `test.deadline`, `test.close_bus`, `TEST_NO_HELLO`); verify it is built by the integration tests and not shipped (`scripts/release.sh` archive lists only `tilrd` and `tilr`)

## 2. Broker and config

- [ ] 2.1 `bus.toml` parsing (D6), reserved names, socket path check (D7); verify `bus::a_bad_bus_toml_leaves_panes_working`, `bus::a_socket_path_that_is_too_long_is_a_config_error`, unit tests in `bus/config.rs`; README "Components"
- [ ] 2.2 Broker routing, `from`, reply check, errors, deadlines, `Cancel`, bounded queues, no global lock (D2-D5); verify `bus::calls_go_round_robin_over_the_providers`, `bus::from_is_always_the_callers_identity`, `bus::a_failed_call_says_why`, `bus::a_full_queue_is_answered_busy`, `bus::a_nested_call_gets_the_time_that_is_left_and_a_cancel`, `bus::a_frame_over_the_limit_closes_the_connection`, `bus::a_slow_instance_does_not_hold_up_the_others`, `bus::payloads_pass_byte_for_byte`; USE_CASES.md entries
- [ ] 2.3 Minimal authorization (D10); verify `bus::calls_are_denied_unless_allowed`

## 3. Runner and control

- [ ] 3.1 Spawn (fd and socket), environment, Hello deadlines, output to the log (D7); verify `bus::an_instance_from_bus_toml_is_started_and_answers`, `bus::an_instance_gets_its_connection_and_names`, `bus::a_socket_instance_connects_once_with_its_token`, `bus::an_instance_that_never_says_hello_is_restarted`, `bus::an_instances_output_is_in_the_server_log`
- [ ] 3.2 Supervision (D8); verify `bus::a_crashed_instance_is_restarted_with_backoff`, `bus::an_instance_that_loses_its_connection_is_restarted`; add the slow ones to `.config/nextest.toml`
- [ ] 3.3 Control methods (D9); verify `bus::an_instance_is_stopped_and_started`, `bus::a_module_is_reloaded_one_instance_at_a_time`, `bus::reloading_bus_toml_applies_the_difference`, `bus::reloading_a_bad_bus_toml_changes_nothing`, `bus::instances_and_methods_are_listed`; USE_CASES.md entries

## 4. Owner access and tilr

- [ ] 4.1 `BusCall`/`BusReply` in `tilr.proto` (D11), bulk lane for replies; README "Protocol" (no feature flag, D12); verify `bus::a_client_calls_the_bus_through_tilrd`, `cargo test -p tilr-proto`
- [ ] 4.2 `tilr bus call|ls|start|stop|reload|reload-config`; verify `client::tilr_bus_call_prints_the_reply`; README "tilr"; USE_CASES.md entry

## 5. tilrd reload

- [ ] 5.1 Drain, `BusState` handover, `bus-state 1` in `state-formats` (D13); verify `reload::bus_instances_survive_a_reload`, `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` updated; README "Reloading without closing shells"

## 6. Checks and archive

- [ ] 6.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 6.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
