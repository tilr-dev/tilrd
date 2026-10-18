# Tasks

## 1. Deadline and cancel with an escaped descendant

- [ ] 1.1 Poll the deadline and cancel first; verify with `exec::a_descendant_that_leaves_the_group_does_not_hold_the_time_limit` (`setsid sleep 60 &` in a shell exec with a 1 s limit: `ExecExit` reason time limit within 3 s)

## 2. Message order and replies

- [ ] 2.1 ExecExit last; verify with `exec::exec_exit_is_the_last_message`
- [ ] 2.2 Refuse ExecStart when exec is off; verify with `exec::exec_start_with_exec_off_is_refused`
- [ ] 2.3 Never drop ExecStarted / ExecInputAck; verify under a full interactive lane in itests UC-077

## 3. No signal after the reap

- [ ] 3.1 Signal the group only before the reap; verify by review and a unit test of the exec state machine

## 4. Checks

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
