# Tasks

## 1. Provider

- [ ] 1.1 Reserved instance `tilrd` with `pane.*` (D1, D2) on the supervisor's existing paths; verify `bus_panes::tilrd_provides_pane_methods`, `bus_panes::a_pane_created_on_the_bus_is_an_ordinary_pane`, `bus_panes::resizing_and_closing_a_pane_on_the_bus`; USE_CASES.md entries
- [ ] 1.2 `pane.write` on the `RawInput` path with its acknowledgement; text snapshot request to the pane process and `pane.screen` waiting on `seq` (D3, backpressure limits); verify `bus_panes::writing_and_reading_a_pane_on_the_bus`, `bus_panes::waiting_for_the_screen_to_change`; unit test for the text rendering in `crates/tilr-screen`

## 2. Rights and creator

- [ ] 2.1 `PaneInfo.created_by` in `tilr.proto`, `supervisor-state` 4 (D5); README "Protocol"; verify `reload::a_bus_created_pane_keeps_its_creator_across_a_reload`, `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into` updated
- [ ] 2.2 Pane rights in tokens (`rights.pane`, `tilr bus token --pane`) and the provider's second authorization (D4); verify `bus_panes::creating_a_pane_on_the_bus_needs_pane_rights`, `bus_panes::pane_access_follows_the_creator`, `bus_panes::a_pane_outlives_the_instance_that_made_it`; audit lines as in `bus-acl`
- [ ] 2.3 README "Components": panes on the bus (methods, rights, an agent example with a sandboxed profile); verify by review against the `bus-panes` spec

## 3. Checks and archive

- [ ] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
