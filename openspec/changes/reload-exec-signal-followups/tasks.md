# Tasks

## 1. SIGPIPE after a failed exec

- [ ] 1.1 After `command.exec()` returns (supervisor, `supervisor.rs:2055`; pane process, `pane.rs:639-641`) set SIGPIPE to `SIG_IGN` again before anything else; verify with a new test in `tests/reload.rs`: a reload whose exec fails (failpoint making `execvp` fail after `do_exec` ran, or an executable that is removed between check and exec), then a write to a closed pipe in the server (e.g. its stderr reader closed) leaves the server running and logging `reload failed`

## 2. A stop before the exec is not lost

- [ ] 2.1 Arm a recording handler for SIGTERM and SIGINT at the last drain in `reload()` (`supervisor.rs:1881`; the `record_held` chain with `HELD_TAKEN` off for the window), and after `hold_everywhere` raise what it recorded on the process so it waits, pending, for the successor; when the exec fails, handle it as an `Event::Shutdown`; verify with a new test `reload::a_sigterm_while_the_reload_prepares_its_exec_stops_the_server` (failpoint `sleep:1500` after the drain, e.g. before `self.tunnel.hold()`; red before: the server keeps running)
- [ ] 2.2 `reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor` waits for a log line written at the `reload-exec` failpoint instead of 500 ms after `reloading`; verify it green 20 times in a row under the full reload suite

## 3. Robustness of the hold

- [ ] 3.1 `spawn_unblocked`: a drop guard re-blocks the signals on the thread, then decrements `spawning` (lock taken with `unwrap_or_else(PoisonError::into_inner)`); `hold_everywhere` waits with `Condvar::wait_timeout` (about 5 s) and on timeout logs an error and the reload is cancelled; verify with a failpoint that panics inside `spawn_unblocked` (a pane spawn), after which a reload goes through or is cancelled with that error, the server keeps serving
- [ ] 3.2 Check on macOS whether the hold needs the exec on the main thread (exec from a tokio worker under a SIGUSR1 flood); write the answer as a comment at the exec site; verify by review
- [ ] 3.3 README reload section and USE_CASES.md; verify by review

## 4. Checks and archive

- [ ] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 4.2 `./itests/dt` then `./itests/dt coverage` exits 0
- [ ] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
