# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. tilr on the bus

- [ ] 1.1 [5.1, tilr] `tilr` (every subcommand, attach with input replay and reconnect) and `tilrd ps` on the Rust client of `tilr-bus` (T10); every test in `crates/tilr/tests/` moved with its name kept; verify `cargo nextest run -p tilr --locked` green with the same names
- [ ] 1.2 [5.4] `tilr bus call|sub|ls|start|stop|reload|reload-config`; verify `client::tilr_bus_call_prints_the_reply`, `client::tilr_bus_sub_prints_events`; README "tilr"; USE_CASES.md entries (own section)
- [ ] 1.3 [3.3, commands] `tilr bus token|revoke|rotate-key`; verify `client::tilr_bus_token_mints_a_token_that_works`
- [ ] 1.4 [7.3] `tilr new --profile`, `tilr profiles`; verify `client::tilr_new_with_a_profile_starts_in_the_profiles_directory`, `client::tilr_profiles_lists_the_profiles_of_the_server`; README "tilr"

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
