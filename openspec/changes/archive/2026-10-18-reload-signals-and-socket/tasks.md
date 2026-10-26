# Tasks

## 1. Signals across the exec

- [x] 1.1 Block SIGUSR1, SIGHUP and SIGTERM in the exec's `pre_exec` (unblock if the exec fails); in the successor record and unblock them first thing in `main`, and act on the record once the supervisor's handlers are registered (design D1); verify with `reload::signals_arriving_while_the_successor_starts_are_held_for_it` (SIGHUP and SIGUSR1 at `supervisor-resume-live=sleep:1500`: server alive, `SIGHUP ignored`, one more reload done, a new shell dies of `kill -HUP $$`), `reload::a_sigterm_while_the_successor_starts_stops_the_server_cleanly` and `reload::a_storm_of_reload_signals_costs_no_shell`

## 2. Socket after a refused reload

- [x] 2.1 Keep the socket and lock serving after a refused or failed reload; a server serving its socket does not reload into an executable without `unix-socket 1` (design D2); verify with `local::a_refused_reload_keeps_the_socket_and_its_lock` (refusal by `supervisor-resume=panic` in the check), `local::a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket` and itests UC-06B (`uc_06b.py::a_refused_reload_keeps_the_socket_and_tilr_finds_the_same_server`: a server `tilr` started, a release that crashes resuming, `tilr ls` and a new `tilr` find the same server, no second daemon)
- [x] 2.2 README "Reloading without closing shells", USE_CASES.md and itests/USE_CASES.md entries for the new tests, spec tags and `./itests/dt coverage`; verify by review against this proposal and coverage exiting 0

## 3. Checks

- [x] 3.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
