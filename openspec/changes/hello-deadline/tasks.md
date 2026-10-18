# Tasks

## 1. Hello deadline

- [ ] 1.1 Close a main-protocol connection that has not been greeted 10 s after its handshake, with 4002 `timeout` and the log line, independent of `--client-timeout`; verify with the new `auth::a_connection_that_never_says_hello_is_closed_after_10_s` (server with `--client-timeout 0`, raw WebSocket answering pings: closed with 4002 in 9-13 s; a client that says `Hello` after 1 s stays)
- [ ] 1.2 itests UC-039: add `a_websocket_that_never_says_hello_is_closed_after_10_s` (`--client-timeout 0`) tagged `@case(spec=["access-control/a-handshake-has-10-seconds#websocket-without-hello"])`; keep `a_websocket_that_never_says_hello_is_dropped_by_the_client_timeout` (4 s client timeout still wins); itests/USE_CASES.md UC-039 text; verify `./itests/dt --only scenarios --filter uc_039`
- [ ] 1.3 README ("Connections" / "Disconnects") and USE_CASES.md entry; verify by review against this proposal

## 2. Checks and archive

- [ ] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 2.2 `./itests/dt --only scenarios --filter uc_039` and `--filter uc_038` green, then `./itests/dt coverage` exits 0
- [ ] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
