# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Reload

- [ ] 1.1 [5.2, internal schemas] Move the internal schemas to `crates/tilrd/proto/internal.proto` (legacy `PaneHandover` fields 16-17 reserved), the client-facing types from `tilr.pane.v1`; `tilr.proto` keeps the `tilr.v1` client messages unchanged; verify every existing reload test green and `rg -n "message (SupervisorState|PaneState|PaneHandover|PaneStatus|PendingInput|PaneInputAck)" crates/tilr-proto` empty
- [ ] 1.2 [6.1] `bus-state 1` (instances, connection and listener fds, provides, counters, tokens' revocation ids and renewal times, subscriptions), drain before the exec, `supervisor-state 3` (profile snapshot, `created_by`, `sandboxed`), `state-formats` (B13); verify `reload::bus_instances_survive_a_reload`, `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` updated (also the refusal from a `supervisor-state 2` build, with its log line, the panes and clients kept), `input_delivery::unread_input_is_replayed_after_reload_in_the_middle_of_a_batch`
- [ ] 1.3 [4.2, 7.2, 8.4 reload tests] What the panes carry: verify `reload::a_bus_created_pane_keeps_its_creator_across_a_reload`, `reload::a_pane_keeps_its_profile_across_a_reload`, `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload` (Linux here; macOS in the `sandbox-macos` job), `reload::an_outside_participant_finds_its_panes_after_a_reload` (a token participant with `access:own`); README "Reloading without closing shells"; USE_CASES.md entries (own section)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
