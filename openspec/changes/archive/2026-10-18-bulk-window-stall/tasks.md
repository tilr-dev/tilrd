# Tasks

## 1. Every ping carries the position

- [x] 1.1 `BulkWindow::position()`; the writer's liveness ping carries it (design D1); verify with a unit test in `outbox.rs` (`outbox::tests::any_pong_moves_the_window`) and the new `files::a_client_answering_only_the_latest_ping_gets_the_whole_file` (a raw test client that holds pongs and answers only the last ping read, `--client-timeout 1`, 16 MiB file arrives whole)
- [x] 1.2 Cap the acknowledged position at `sent` (design D3); verify with `outbox::tests::a_pong_beyond_what_was_sent_opens_no_more_than_the_window`
- [x] 1.3 USE_CASES.md entry for the files test; README "Protocol" (bulk window paragraph) says every server ping carries the position

## 2. No lost wake-up for a lagging client

- [x] 2.1 Ask for the full frame at lag start when the lane is already drained, and on every `Tick` for drained lagging clients (design D2); verify with the new `reconnect::a_lagging_client_gets_the_last_screen_without_new_output` (client with a small receive buffer stops reading during heavy output, the pane goes quiet, client reads: last frame is full and matches the screen) and `metrics::a_client_that_lags_skips_to_the_latest_screen_and_still_gets_its_echo` unchanged
- [x] 2.2 USE_CASES.md entry

## 3. Checks and archive

- [x] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 3.2 `./itests/dt --only scenarios --filter uc_049`, then `uc_072`, `uc_073`, `uc_043` green, then `./itests/dt coverage` exits 0
- [x] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
