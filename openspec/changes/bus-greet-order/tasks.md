# Tasks

## 1. The greeting's order

- [ ] 1.1 Register a greeted connection (connection, providers, `HelloAck` queued) under one hold of the broker's lock, then announce it up and log `participant greeted`; failpoint `bus-greeted:<client name>` after; verify tests `bus::a_participant_is_callable_once_it_has_its_hello_ack`, `bus::a_started_instance_is_callable_and_listed_when_bus_start_returns` (red before, green after); USE_CASES.md entries
- [ ] 1.2 Bus tests that read the log or order events across connections wait on an event: `bus::an_instance_is_stopped_and_started` waits for `instance t-1 is stopped`, `bus::events_reach_the_subscribers_connected_now` has an earlier subscriber receive `old` before the late subscription

## 2. Checks and archive

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, the bus tests with `--features tilrd/failpoints`, `cargo deny check`, `openspec validate --all --strict`; the Linux stress run of the bus tests that raced the greeting; verify all green
- [ ] 2.2 Archive the change; verify `openspec validate --all --strict`
