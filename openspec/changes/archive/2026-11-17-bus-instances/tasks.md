# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former single change `unified-bus` (shared design, O3).

## 1. Instances

- [x] 1.1 [2.2] `bus.toml` (B10) with `rights` parsed into `Rights`, runner (spawn, fd and instance socket, environment, output to the log, Hello deadlines), supervision with backoff and restart on a lost connection, control methods, the interim reload of the design; verify `bus::an_instance_from_bus_toml_is_started_and_answers`, `bus::a_bad_bus_toml_leaves_panes_working`, `bus::reloading_a_bad_bus_toml_changes_nothing`, `bus::a_socket_path_that_is_too_long_is_a_config_error`, `bus::an_instance_gets_its_connection_and_names`, `bus::a_socket_instance_connects_once_with_its_token`, `bus::a_crashed_instance_is_restarted_with_backoff`, `bus::an_instance_that_loses_its_connection_is_restarted`, `bus::an_instance_that_never_says_hello_is_restarted`, `bus::an_instance_is_stopped_and_started`, `bus::a_module_is_reloaded_one_instance_at_a_time`, `bus::reloading_bus_toml_applies_the_difference`, `bus::participants_and_methods_are_listed`, `bus::an_instances_output_is_in_the_server_log`; slow tests added to `.config/nextest.toml`; README "The bus" instances subsection; USE_CASES.md entries (own section)
- [x] 1.2 [2.1, 2.3] Instance transports on the broker: `bus::every_transport_speaks_the_same_envelopes` (fd, socket, WebSocket owner), `bus::a_frame_over_the_limit_closes_the_connection`, the instance case of `bus::reserved_namespaces_cannot_be_provided`
- [x] 1.3 On the broker's deadlines: `call_timeout_ms` handed to `BrokerHandle::set_call_timeout` at start and on `bus.reload_config`, the methods of configured instances made known (`know_methods`); verify `bus::call_timeout_ms_of_bus_toml_bounds_every_call`, `bus::a_method_of_a_configured_instance_is_unavailable_while_it_is_down`

## 2. Checks and archive

- [x] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [x] 2.2 [12.2] Archive the change and list it under Done in `openspec/ROADMAP.md`; verify `openspec validate --all --strict`
