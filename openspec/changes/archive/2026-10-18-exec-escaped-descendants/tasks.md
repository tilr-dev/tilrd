# Tasks

## 1. The drain is bounded and cancellable

- [x] 1.1 `pump()`: drop `biased`, keep a 1 s budget counted only while waiting on or reading the pipe once `over` is set (design D1); verify with the new `exec::a_process_outside_the_group_that_keeps_writing_does_not_hold_the_exec` (escaped `yes`: `ExecExit` within 3 s; 64 more execs start afterwards); also a 1 MiB cap on what is read after the group is gone (design D1, corrected)
- [x] 1.2 `run()`: await the readers in a `select!` with control and deadline; `ExecCancel`, time limit and client gone abort them; verify with the "Cancel during the drain" scenario in the same test file (`exec::a_cancel_during_the_drain_ends_the_exec_at_once`)
- [x] 1.3 `exec::what_a_program_leaves_running_in_its_group_goes_when_it_exits` and `exec::a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client` still pass (tail of a dead program reaches a slow client); `cargo test -p tilrd --test exec`
- [x] 1.4 Kill the group before the reap (design D4); verify with a unit test in `exec.rs` that the leader is still a zombie (`/proc/<pid>/stat` state `Z` on Linux, `ps -o stat=` on macOS) when the group kill is sent, as `exec::tests::the_group_is_killed_while_the_program_is_a_zombie` (on the wait-without-reaping step the run uses), and by review
- [x] 1.5 USE_CASES.md entries for the new tests; spec `exec` Notes drop "the 1 s drain of a pipe held by a process outside the group" from the untested list

## 2. ExecExit is last

- [x] 2.1 `ToClient::queue_bulk_now` (bulk packet without waiting for credit) with a unit test in `outbox.rs` (`outbox::tests::a_packet_queued_now_goes_after_the_bulk_before_it`)
- [x] 2.2 `Run` keeps the reader and stdin abort handles; `stop_all` aborts them, then queues the stand-in with `queue_bulk_now` (design D2); verify with `exec::an_exec_given_up_at_a_reload_ends_with_its_exec_exit` (client stops reading a `yes` exec, SIGUSR1, then reads: last message of that exec is `ExecExit` `reason` `"reload"`)
- [x] 2.3 USE_CASES.md entry

## 3. Replies

- [x] 3.1 Exec off answers `ExecStarted { error: "exec is turned off" }` (design D3); update `exec::exec_is_offered_unless_turned_off` to expect it before `Pong`
- [x] 3.2 `ExecStarted` / `ExecInputAck` that `try_send` cannot queue report the client stuck and the supervisor drops it with 4001; verify with `exec::tests::a_reply_the_lane_cannot_take_drops_the_client`
- [x] 3.3 README "Exec" says what an `ExecStart` with exec off gets; itests UC-077: a new case that `ExecStart` to a `--no-exec` server is answered with the error, tagged `@case(spec=["exec/exec-is-offered-unless-turned-off"])`; verify `./itests/dt --only scenarios --filter uc_077`

## 4. Checks and archive

- [x] 4.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [x] 4.2 `./itests/dt --only scenarios --filter uc_077` green, then `./itests/dt coverage` exits 0 and `openspec/coverage/` is regenerated
- [x] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
