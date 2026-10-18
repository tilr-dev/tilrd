# Tasks

## 1. No thread takes a held signal during the exec

- [x] 1.1 `reload.rs`: `serve_signals()` (block the four on the calling thread, start the `signals` thread that lets them in), `hold_everywhere()` / `release_everywhere()`; `main.rs` calls `serve_signals()` for the server command after `record_signals()` and before the runtime; `supervisor.rs` calls `hold_everywhere()` before `command.exec()` and `release_everywhere()` after a failed one; verify with the new `reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor` (failpoint `reload-exec=sleep:1500`; red before the change: the server keeps running)
- [x] 1.2 `spawn_unblocked` for the pane process, the successor checks, `exec` programs and `cloudflared`; verify with `reload::signals_arriving_while_the_successor_starts_are_held_for_it` (a shell started afterwards dies of `kill -HUP $$`) and `exec::the_signal_that_killed_a_program_is_reported` (SIGINT reaches an `exec` program)
- [x] 1.3 A SIGUSR1 flood (0.2 ms apart, 10 s, macOS) no longer kills the server: 0 of 13 runs after the change, 3 of 5 before
- [x] 1.4 README reload section and USE_CASES.md; verify by review

## 2. Checks and archive

- [x] 2.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; `cargo test -p tilrd --features tilrd/failpoints --test reload` 10 times in a row green (known flakes rerun)
- [x] 2.2 `./itests/dt` then `./itests/dt coverage` exits 0
- [x] 2.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
