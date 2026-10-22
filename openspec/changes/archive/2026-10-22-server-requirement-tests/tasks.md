# Tasks

## 1. Server

- [x] 1.1 `exec::input_that_is_not_wanted_is_ignored`, `metrics::the_timing_fields_measure_what_they_say`, `reload::stale_state_files_are_swept_at_start` (files older than an hour via `touch -t`), each green; USE_CASES.md entries
- [x] 1.2 `panes::a_pane_whose_command_cannot_start_is_reported_to_its_creator` and `panes::used_at_ms_follows_subscribe_and_input`; USE_CASES.md entries
- [x] 1.3 `socks::the_socks_path_is_not_served_on_the_unix_socket` and `socks::a_session_that_does_not_say_where_it_goes_is_closed` (advances the test clock by 10 s); `agent::a_65th_stream_is_refused`; USE_CASES.md entries

## 2. Checks

- [x] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 2.2 Refresh each MODIFIED requirement in `specs/` from the current `openspec/specs/` if a change archived since touched it; drop "(planned …)" from the **Verified by** lines; remove the tested items from the specs' Notes; verify `openspec validate server-requirement-tests --strict`
