# Tasks

## 1. Reconnect

- [x] 1.1 Errors from `list_panes` / `subscribe` after a reconnect go to the failed-attempt path (status line, backoff); `PaneGone` and `Untrusted` unchanged; verify with the new `client::an_error_right_after_a_reconnect_is_retried` (a stand-in server on a TCP port closes the second connection on `ListPanes`; `tilr` stays attached and typing reaches the pane)
- [x] 1.2 The connect attempt is a pinned future polled beside input in the disconnected loop; Ctrl+] d drops it; verify with the new `client::detach_works_while_a_reconnect_attempt_hangs` (a stand-in TCP listener that accepts and never answers; `[detached from pane …]` within 1 s of the keys, the connect timeout being 5 s)
- [x] 1.3 USE_CASES.md entries

## 2. Signals

- [x] 2.1 SIGTERM, SIGHUP, SIGINT handled while attached (an async-signal-safe handler, see proposal Impact): restore the terminal, then reset to the default action and re-raise; verify with the new `client::a_terminated_attach_restores_the_terminal` (pty-attached `tilr`, `kill -TERM`: exit by signal 15, the pty's modes (tcgetattr, as `stty -a` reads them) have `icanon` and `echo`, pane still in `tilr ls`)
- [x] 2.2 README `tilr` section ("Disconnects" / detaching) says what a signal does; USE_CASES.md entry; verify by review against this proposal

## 3. Checks and archive

- [x] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 3.2 `./itests/dt --only scenarios --filter uc_010` and `--filter uc_014` green (no case added: the cargo tests drive `tilr` in a pty), then `./itests/dt coverage` exits 0
- [x] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
