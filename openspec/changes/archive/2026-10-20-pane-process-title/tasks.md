# Tasks

## Supervisor follow-up (2026-10-20)

- [x] Reproduce and fix a foreground leader's exec within the same group; re-read cmdline on the one-second tick and cover the silent exec with a FIFO-controlled integration test.
- [x] Reproduce and fix loss of the contiguous environment prefix when a later entry has moved; preserve all environment values and original argv.
- [ ] Share the title-test setup between panes and reload; run narrow tests, self-review and bin/finish checks.

## 1. Keep the start arguments

- [x] 1.1 Take the pane process's start arguments once at the start of `pane::run` (before the runtime) and use them for `reload::check_successor` and the exec in `exec_successor` instead of `std::env::args_os()`; verify `cargo test -p tilrd --test reload` (existing pane reload tests stay green) and `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps`

## 2. Title area

- [x] 2.1 `crates/tilrd/src/title.rs`: copy the environment, repoint `environ`, find the argv + environment area (D2), write a title cut to the area and padded with NULs; Linux and macOS, warn once when there is no room; verify with unit tests `title::tests::a_title_is_cut_to_the_area` and `title::tests::the_environment_is_unchanged_after_the_move` (an env var read before and after)
- [x] 2.2 Set the title `[shell <pid> in <cwd>]` when the pane starts and when `PaneStatus.cwd` changes; verify with `panes::the_pane_process_title_follows_the_directory_and_the_command` (part: `cd /tmp`, read with `common::process_args`) and update `panes::a_pane_is_told_apart_by_its_id_in_its_shell_and_in_ps` to compare the part before ` [`

## 3. Foreground command

- [x] 3.1 `tcgetpgrp` after pty output and on a 1 s tick; leader command line from `/proc/<pgid>/cmdline` (Linux) or `KERN_PROCARGS2` (macOS), 80 bytes at most; rewrite only on change (D3); verify with `panes::the_pane_process_title_follows_the_directory_and_the_command` (part: `sleep 30`, then Ctrl+C)
- [x] 3.2 After a reload the successor sets the title again; verify with `reload::the_pane_process_title_is_set_again_after_a_reload`
- [x] 3.3 README "Telling panes apart" describes the title; USE_CASES.md lists the two new tests; verify by review against this proposal

## 4. Checks and archive

- [x] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [x] 4.2 `./itests/dt --only scenarios --filter uc_001`, `./itests/dt --only scenarios --filter uc_060`, and `./itests/dt --only scenarios --filter uc_06a` (cases that find pane processes by ` pane --id `) green, then `./itests/dt coverage` exits 0
- [x] 4.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
