# Tasks

## 1. The peer check for the host socket

- [x] 1.1 `tilr-proto::local`: a helper that reads a Unix stream's peer uid and compares it with the euid, returning the message of the proposal (the comparison in a pure function taking both uids); `tilr`'s `check_peer` (`crates/tilr/src/conn.rs:349-369`) uses it, its error text unchanged; verify with the new unit test `local::tests::a_peer_of_another_uid_is_refused` (crates/tilr-proto/src/local.rs; own uid passes, uid + 1 fails with the message) and the existing `client::` tests of the socket check still green
- [x] 1.2 `ask` in `crates/tilrd/src/host.rs` (the pane's `tilr`, `tilrd edit` / `tilrd open`) and in `crates/tilr/src/host.rs` (the client) run the check after `connect()` and before the write; a refusal goes out as `tilr: cannot reach the tilr server: …`, exit 1; verify with `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file` and `client::tilr_edit_in_a_pane_asks_the_server_of_the_pane` still green (own uid), and 1.3
- [x] 1.3 itests case `uc_03a.py::tilr_edit_says_nothing_to_a_host_socket_another_user_serves` (mallory serves the fake socket of `uc_03a.py`; alice runs `tilr edit`, `tilr open` and `tilrd edit` with `TILR_PANE` and `TILR_SOCK` naming it; each exits 1 with `is served by another user (uid N)`, every connection got 0 bytes), tagged `@case(spec=["clipboard-host-events/tilr-edit-and-tilr-open-talk-only-to-a-host-socket-of-the-user-s#another-user-s-socket-at-tilr-sock"])`; write it first and see it red; verify `./itests/dt --only scenarios --filter uc_03a` green after 1.2
- [x] 1.4 itests/USE_CASES.md UC-03A bullet names the new case; USE_CASES.md entry; README "Files and `tilr edit`" (`README.md:507-540`) says `tilr edit` checks who serves `$TILR_SOCK`; verify by review against this proposal

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 2.2 `./itests/dt --only scenarios --filter uc_03a` and `--filter uc_035` green, then `./itests/dt coverage` exits 0 (the new tag names the added requirement)
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
