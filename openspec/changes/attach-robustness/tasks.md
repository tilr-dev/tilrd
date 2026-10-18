# Tasks

## 1. Reconnect

- [ ] 1.1 Errors from `list_panes` / `subscribe` after a reconnect go to the failed-attempt path (status line, backoff); `PaneGone` and `Untrusted` unchanged; verify with the new `client::an_error_right_after_a_reconnect_is_retried` (the server reloads a second time while `tilr`'s reconnect waits for `ListPanes`, e.g. with a failpoint delay in the live resume; `tilr` stays attached and typing reaches the pane)
- [ ] 1.2 The connect attempt is a pinned future polled beside input in the disconnected loop; Ctrl+] d drops it; verify with the new `client::detach_works_while_a_reconnect_attempt_hangs` (socket replaced by a test listener that accepts and never answers; `[detached from pane …]` within 200 ms of the keys)
- [ ] 1.3 USE_CASES.md entries

## 2. Signals

- [ ] 2.1 SIGTERM, SIGHUP, SIGINT streams in the attach loop: restore the terminal, then reset to the default action and re-raise; verify with the new `client::a_terminated_attach_restores_the_terminal` (pty-attached `tilr`, `kill -TERM`: exit by signal 15, `stty -a` on the pty shows `icanon` and `echo`, pane still in `tilr ls`)
- [ ] 2.2 README `tilr` section ("Disconnects" / detaching) says what a signal does; USE_CASES.md entry; verify by review against this proposal

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 3.2 `./itests/dt --only scenarios --filter uc_010` and `--filter uc_014` green (no case added: the cargo tests drive `tilr` in a pty), then `./itests/dt coverage` exits 0
- [ ] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
