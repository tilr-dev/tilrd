# Tasks

## 1. The executable opened once

- [ ] 1.1 `reload.rs`: a `Checked` executable (Linux: descriptor, path `/proc/self/fd/N`, fallback to the copy without `/proc/self/fd`; macOS: clone or copy into `<runtime dir>/exe/<pid>/<name>`, 0700), with the identity (`ino`, size, mtime) of what was opened; `check_executable` and `check_successor` run it; dropping it closes the descriptor or removes the copy; verify with new unit tests in `reload.rs` (the copy is 0700 in a 0700 directory and gone after drop; the identity is the opened file's even after the path is replaced)
- [ ] 1.2 `TILR_RELOAD_EXE=<ino>:<size>:<mtime>:<nsec>:<path>`: set on the exec and on the check children; `main` reads and removes it before anything else, `exe()` returns its path (else `current_exe()`), the watch starts from its identity; on Linux `PR_SET_NAME` to the installed file name; the successor removes its `exe/<pid>/`; a start sweeps `exe/<pid>/` of dead pids; verify with a unit test of the parse (bad values fall back to `current_exe()`) and 1.4
- [ ] 1.3 Supervisor reload (`supervisor.rs` `reload` and `exec_successor`) opens once and uses the `Checked` file for `state-formats`, the check and the exec (`arg0` = installed path); verify with the new `reload::the_executable_checked_is_the_one_execd` (`reload-before-exec=sleep:1500`, replace with a script that appends its arguments to a file and exits 1; same pid, panes alive, the file has only `state-formats` lines, later `not reloading` in the log)
- [ ] 1.4 `reload::a_reload_shows_in_no_command_line_and_leaves_no_file_behind` also checks `ps -o comm` of the supervisor and the pane process (`tilrd`) and that no `exe/` copy is left; verify that test
- [ ] 1.5 USE_CASES.md entries; README "Reloading without closing shells" says the checked file is the one exec'd, what `ps` shows, and the macOS rollback caveat (design, "Reload and reconnect"); verify by review against this proposal

## 2. Pane processes

- [ ] 2.1 `pane.rs` reload opens once and uses the `Checked` file for its check and exec; failpoint `pane-reload-checked` between them; verify with the new `reload::a_pane_execs_the_executable_it_checked` (`pane-reload-checked=sleep:1500` via `TILR_FAILPOINTS_FILE`, replace with the script; pane pid and shell pid unchanged, the script has only `state-formats` lines)
- [ ] 2.2 `reload::an_executable_replaced_again_during_a_reload_costs_no_shell` and the other `reload::` tests still green on Linux and macOS; USE_CASES.md entry; verify `cargo test --features tilrd/failpoints --test reload`

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 3.2 `./itests/dt --only scenarios --filter uc_003` and `--filter uc_060` green (reloads through an install and an update); no itests tag names the added or changed requirements; `./itests/dt coverage` exits 0
- [ ] 3.3 Record the commit in proposal.md ("Implemented by") and archive the change; verify `openspec validate --all --strict`
