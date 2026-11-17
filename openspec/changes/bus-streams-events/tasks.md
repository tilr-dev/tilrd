# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Bounds of the broker

- [ ] 1.1 [2.1] Bounded queues and lanes on the existing `outbox.rs`, `max_in_flight`, `busy`, deadlines and `Cancel`, the full error set, no global lock, one timer wheel (B5, B8); verify `bus::a_failed_call_says_why`, `bus::a_full_queue_is_answered_busy`, `bus::a_nested_call_gets_the_time_that_is_left_and_a_cancel`, `bus::a_slow_participant_does_not_hold_up_the_others`
- [ ] 1.2 [2.1] Credits per stream and per connection, overrun ended with `bad_reply`, bulk lane bounded, interactive first (B7); verify `bus::streams_are_paced_by_the_readers_credit`, `bus::a_stream_that_overruns_its_credit_is_ended`, `bus::bulk_data_in_the_network_is_bounded_per_connection`, `bus::interactive_traffic_goes_ahead_of_bulk_traffic`, unit tests `interactive_packets_go_ahead_of_bulk_ones` and `bulk_waits_for_room_counted_in_bytes` still green; README "The bus" streams subsection
- [ ] 1.3 [2.1] Events dropped for laggards and counted (B6); verify `bus::a_lagging_subscriber_loses_events_not_its_connection`; metric `tilrd_bus_events_dropped_total` in README "Metrics"; USE_CASES.md entries (own section); slow tests added to `.config/nextest.toml`

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
