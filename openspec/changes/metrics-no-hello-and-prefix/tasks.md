# Tasks

## 1. no_hello

- [ ] 1.1 `ClientDrop::NoHello` (label `no_hello`, last in `ALL`); `close_ungreeted` drops with `Gone::Dropped(ClientDrop::NoHello)`, keeping its log line and 4002 `timeout`; verify with the unit test `metrics::tests::exposition_lists_every_family_with_help_and_type` (a `no_hello` sample of 0) and the new `metrics::a_connection_that_never_says_hello_is_counted_as_no_hello` (WebSocket opened, no `Hello`, closed 4002 after ~10 s; `no_hello` 0 → 1, the other reasons 0)
- [ ] 1.2 `metrics::metrics_count_the_clients_let_go_and_why` also checks that a `Hello` with a wrong token changes no drop sample; `auth::a_connection_that_never_says_hello_is_closed_after_10_s` still green; USE_CASES.md entry; verify those tests

## 2. One prefix

- [ ] 2.1 Rename the three gauges `tilrd_info`, `tilrd_clients`, `tilrd_panes` to `tilr_info`, `tilr_clients`, `tilr_panes` in `metrics.rs` (and its unit tests, `metrics.rs:546-610`) and `crates/tilrd/tests/metrics.rs`; verify `cargo test --features tilrd/failpoints --test metrics` and the `metrics::tests::` unit tests, plus a new assertion in `metrics::metrics_count_frames_input_and_routing` that no line of the body starts with `tilrd_` or `# TYPE tilrd_`
- [ ] 2.2 itests: none reads `tilrd_info`, `tilrd_clients` or `tilrd_panes` (`uc_043.py`, `uc_049.py`, `uc_052.py` read `tilr_*` names that stay), so none changes; verify `grep -rn 'tilrd_\(info\|clients\|panes\)' itests` finds nothing and `./itests/dt --only scenarios --filter uc_052` stays green
- [ ] 2.3 README "Metrics" table with the new gauge names, the `no_hello` reason, and a "Renamed" list (the three, old → new) marked breaking; USE_CASES.md wording; verify `grep -rn 'tilrd_\(info\|clients\|panes\)' README.md USE_CASES.md crates itests` finds only the rename list

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 3.2 `./itests/dt coverage` exits 0 (no tag names a changed requirement slug)
- [ ] 3.3 The commit is a `feat!:` / `BREAKING CHANGE:` naming the three renamed gauges; record it in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
