# Tasks

## 1. Wrappers

- [ ] 1.1 Write the zsh, bash and fish wrappers under `<runtime dir>/shell/` with the shims (design D5: temp file and rename, 0700 directory, a log line when they cannot be written); verify with unit tests in `crates/tilrd/src/clipboard.rs` (or a new `shell.rs`): `shell::tests::the_wrappers_are_written_private_and_replaced_whole` and `shell::tests::no_wrappers_means_shells_start_as_before`
- [ ] 1.2 `pty.rs` starts zsh with `ZDOTDIR` (saving the server's in `TILR_ZDOTDIR`), bash as `-bash --posix` with `ENV`, fish as `-fish --login --init-command`, others as today (design D1-D4); verify with unit tests `pty::tests::a_login_shell_is_wrapped_by_its_name` (argv and environment per shell, a command never wrapped) and the existing `pty::tests::a_login_shell_is_started_under_its_name_with_a_dash`
- [ ] 1.3 On macOS (zsh and bash in the base system): `panes::a_pane_without_a_command_runs_the_users_login_shell` still passes with the wrapper in place; verify `cargo test -p tilrd --test panes` on macOS and Linux

## 2. Black-box case

- [ ] 2.1 itests runtime image installs `zsh` and `fish` (`RUNTIME_APT` in `itests/src/tilr_itests/main.py`); verify `./itests/dt` builds the image
- [ ] 2.2 New case UC-07B (`itests/scenarios/uc_07b.py`, itests/USE_CASES.md group G next to UC-076): users whose login shells are bash, zsh, fish and dash, each with a profile that resets `PATH` and sets `X=1`; cases `the_tools_stay_first_on_path_when_the_profile_resets_it` and `the_users_startup_files_still_run`, tags `panes/the-tools-stay-first-on-path-after-the-login-profile#bash-with-a-profile-that-resets-path`, `#zsh-and-fish`, `#the-user-s-files-still-run`, `#another-shell`; verify `./itests/dt --only scenarios --filter uc_07b` green
- [ ] 2.3 README pane environment and "Pasting images" say the shims stay first for zsh, bash and fish and keep the `TILR_SHIMS` hint for other shells; USE_CASES.md lists the unit tests; the `clipboard-host-events` comment in `clipboard.rs:30-34` says the same; verify by review

## 3. Checks and archive

- [ ] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`
- [ ] 3.2 `./itests/dt --only scenarios --filter uc_07b` and `--filter uc_075` (clipboard shims) and `--filter uc_076` (`tilr` shim) green; `./itests/dt coverage` exits 0
- [ ] 3.3 Record the commit in proposal.md and archive the change
