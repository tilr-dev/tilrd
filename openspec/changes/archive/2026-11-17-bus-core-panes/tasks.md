# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former single change `unified-bus` (shared design, O3).

## 1. Panes on the bus

- [x] 1.1 [4.1] `crates/tilrd/src/bus/core/panes.rs`: the participant `tilrd` with `pane.create|list|close|resize|input|applied|input_finalize`, `pane.output` (frames forwarded unchanged, latest-wins on lag or spent credit, `End` on exit), `pane.read`, the pane events with scope (T2, T4, T5, T9) on the supervisor's existing paths through the handle; `seq_base` from `connection_id`; `PaneInfo.created_by` stamped; verify `bus_terminal::a_pane_is_created_listed_and_closed_by_calls`, `bus_terminal::pane_changes_are_events`, `bus_terminal::watching_a_pane_is_a_stream_of_frames`, `bus_terminal::a_lagging_watcher_skips_to_the_latest_screen`, `bus_terminal::input_is_acknowledged_after_it_is_applied`, `bus_terminal::reading_a_pane_and_waiting_for_a_change`; unit test for the text rendering in `crates/tilr-screen`; every existing `tilr.v1` test green unchanged
- [x] 1.2 [2.1] `bus/core/json.rs`: JSON for the core's methods (B9); verify `bus::payloads_pass_byte_for_byte`, `bus::core_methods_answer_json_to_json`, and the `tilrd` clause of `bus::participants_and_methods_are_listed`; USE_CASES.md entries (own section) *(the listing clause was added once `bus-instances` brought the test; see design.md, Implementation notes)*

- [x] 1.3 After `bus-streams-events`: `pane.output` chunks at most the stream's window (images first, a frame over it `too_large`), a frame whole or skipped, a full frame asked for once there is credit for the chunk that did not fit (`caught_up_for`); sessions forgotten on `watch_connection`; verify `supervisor::bus::core::panes::tests::a_frame_is_split_to_the_streams_window`, `bus_terminal::a_slow_reader_of_pane_output_holds_up_no_other_pane_or_reader`, `bus_terminal::a_pane_call_past_its_deadline_is_answered_timeout_and_cancelled` and the scenario "Deadline shorter than the wait" in `bus_terminal::reading_a_pane_and_waiting_for_a_change`; the profile seam named in design.md (wired by `pane-profiles`)

## 2. Checks and archive

- [x] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [x] 2.2 [12.2] Archive the change and list it under Done in `openspec/ROADMAP.md`; verify `openspec validate --all --strict`
