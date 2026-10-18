# Tasks

## 1. Signals while the server starts

- [x] 1.1 `reload.rs`: `HELD` gains SIGINT, `HeldSignals` gains `interrupt`; a `record_signals()` that installs `record_held` for the set whatever the mask (the reload path keeps unblocking what arrived blocked); verify with the unit tests of `reload.rs` and the cargo tests below
- [x] 1.2 `main.rs`: for the server command, call it after `Cli::parse()` and before `supervisor::run` (single-threaded; no tokio runtime yet); verify by review that no thread is started before it (tracing's fmt subscriber starts none)
- [x] 1.3 `supervisor.rs`: failpoint `supervisor-before-signals` before the handlers; a `SignalKind::interrupt()` stream created with `term` and passed to `shutdown_signal` (no `ctrl_c()`); held `terminate`/`interrupt` send `Event::Shutdown` with the log lines of design D4; verify with the new `local::a_sigterm_while_the_server_starts_stops_it_cleanly` and `local::a_sigint_while_the_server_starts_stops_it_cleanly` (failpoint `sleep:1000`, wait until the socket accepts, signal; exit 0, `shutting down` in the log, socket and lock gone) and the SIGHUP variant as a test of its own, `local::a_sighup_while_the_server_starts_is_ignored` (server keeps serving, `SIGHUP ignored`)
- [x] 1.4 Run `local::a_server_that_stops_removes_its_socket_and_lock` 200 times under load (`cargo test --features tilrd/failpoints --test local -- a_server_that_stops --test-threads 1` in a loop while the full suite runs); verify no failure
- [x] 1.5 USE_CASES.md entries for the two tests; README "Signals" (`README.md:268-272`) says a signal while the server starts is handled once it is up; verify by review against this proposal

## 2. SIGINT held across a reload

- [x] 2.1 The successor acts on a held SIGINT like SIGTERM; verify with the new `reload::a_sigint_while_the_successor_starts_stops_the_server_cleanly` (`supervisor-resume-live=sleep:1500`, SIGINT during the sleep: exit 0, `shutting down`, shells end, socket and lock removed)
- [x] 2.2 README reload section (`README.md:668`) names SIGINT among the held signals; USE_CASES.md entry; verify by review

## 3. Checks and archive

- [x] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 3.2 No itests case cites the changed or added requirements (no tags to update); `./itests/dt coverage` exits 0
- [x] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
