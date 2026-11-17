# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. Linux sandbox

- [ ] 1.1 [8.1] Linux: start-up check (Landlock ABI ≥ 4, seccomp), `sandbox` feature, per-pane warning below ABI 6 (S5, S9); `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced` on the failpoint; verify `sandbox::sandbox_is_offered_where_it_can_be_enforced` (failpoints `sandbox-unavailable`, `sandbox-abi-4`)
- [ ] 1.2 [8.2] Linux: Landlock ruleset and seccomp filter (`AF_UNIX` always, UDP with `none`) in the shell's `pre_exec`, base grants, `executables` resolution; verify `sandbox::a_sandboxed_profile_starts_a_sandboxed_pane`, `sandbox::a_sandboxed_pane_writes_only_where_it_may`, `sandbox::a_child_of_the_shell_is_restricted_too`, `sandbox::only_granted_programs_run`, `sandbox::a_script_and_a_binary_made_inside_run_inside`, `sandbox::network_follows_the_profile`, `sandbox::a_command_from_the_request_runs_in_the_profiles_sandbox`; USE_CASES.md entries (own section)
- [ ] 1.3 [8.3] Environment from scratch, no wrappers or shims, refused grants over the state, runtime and configuration directories (S6), fail closed (S7); verify `sandbox::a_sandboxed_pane_has_only_its_variables`, `sandbox::a_sandboxed_pane_cannot_reach_the_server`, `sandbox::a_grant_over_the_state_directory_is_refused`, `sandbox::a_sandbox_that_cannot_be_applied_starts_nothing` (failpoint `sandbox-apply`)
- [ ] 1.4 [8.4] Terminal behaviour and cleanup (S2, S8); verify `sandbox::raw_mode_and_job_control_work_in_a_sandboxed_pane`, `sandbox::a_sandboxed_panes_leftovers_are_removed`, itests UC-0S1 (`uc_0s1.py::vim_less_and_claude_run_in_a_sandboxed_pane`, spec tag `pane-sandbox/a-sandboxed-pane-is-a-normal-terminal`), then `./itests/dt coverage` exits 0 (local, not committed); itests/USE_CASES.md entry
- [ ] 1.5 [8.9, Linux and common] README "Sandboxed panes" (new section): what is always granted, what is refused, limits (UDP with `client` on Linux, `AF_UNIX`, kernels without signal scoping and the log line); verify by review against the `pane-sandbox` spec

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh` (the kernel must have Landlock ABI ≥ 4; note it if not); verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
