# Tasks

## 1. Positions in every ping while closed

- [ ] 1.1 Put the position in every ping while the window is closed; verify with a test client that answers only the newest ping (`files::a_client_answering_only_the_latest_ping_gets_the_whole_file`)

## 2. Wake-up of the last frame

- [ ] 2.1 Wake the writer when a lagging client's full frame is queued; verify with `reconnect::a_lagging_client_gets_the_last_screen_without_new_output`

## 3. Cap pong positions

- [ ] 3.1 Cap at bytes sent; verify with a unit test in `outbox.rs` and itests UC-073 unchanged

## 4. Checks

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
