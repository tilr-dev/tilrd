# Tasks

Active since 2026-11-17. Groups 1 and 2 need `pane-profiles` archived. History: the 2026-10-18
plan was "1. Experiment (quarantine), 2. Mechanism"; its experiment is answered (design D1), its
2.1 (deltas, `skip_specs` dropped) is done in this update.

## 0. Plan

- [x] 0.1 Spec deltas written, `skip_specs` dropped; verify `openspec validate sandboxed-pane --strict`
- [x] 0.2 macOS mechanism decided by the owner (Seatbelt, design D10, 2026-11-17); verify by review that every `pane-sandbox` requirement has a mechanism on macOS

## 1. Linux

- [ ] 1.1 Start-up check (Landlock ABI ≥ 4, seccomp) and the `sandbox` feature (D9); `PaneInfo.sandboxed`; verify with `sandbox::sandbox_is_offered_where_it_can_be_enforced` (failpoint `sandbox-unavailable`); README "Protocol" features table
- [ ] 1.2 Landlock ruleset and seccomp filter in the shell's `pre_exec` (D5), base grants, `executables` resolution; verify with `sandbox::a_sandboxed_profile_starts_a_sandboxed_pane`, `sandbox::a_sandboxed_pane_writes_only_where_it_may`, `sandbox::a_child_of_the_shell_is_restricted_too`, `sandbox::only_granted_programs_run`, `sandbox::a_script_and_a_binary_made_inside_run_inside`, `sandbox::network_follows_the_profile`; USE_CASES.md entries
- [ ] 1.3 Environment from scratch, no wrappers or shims, grant refusals (D6); verify with `sandbox::a_sandboxed_pane_has_only_its_variables`, `sandbox::a_sandboxed_pane_cannot_reach_the_server`, `sandbox::a_grant_over_the_state_directory_is_refused`; USE_CASES.md entries
- [ ] 1.4 Fail closed (D7) with failpoint `sandbox-apply`; verify with `sandbox::a_sandbox_that_cannot_be_applied_starts_nothing`
- [ ] 1.5 Terminal behaviour; verify with `sandbox::raw_mode_and_job_control_work_in_a_sandboxed_pane` and itests UC-0S1 (`uc_0s1.py::vim_less_and_claude_run_in_a_sandboxed_pane`, spec tag `pane-sandbox/a-sandboxed-pane-is-a-normal-terminal`), then `./itests/dt coverage` exits 0
- [ ] 1.6 Temporary directory and sweep (D8); verify with `sandbox::a_sandboxed_panes_leftovers_are_removed`
- [ ] 1.7 Reload; verify with `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload`; README "Reloading without closing shells"

## 2. macOS (Seatbelt, D10)

- [ ] 2.1 Profile generation: SBPL with fixed rules and `(param …)` references only, paths passed as `-D` (no interpolation; NUL/newline refused), the always-allowed rule set of D10, `read_only`/`read_write`/`network` mapping, Unix-socket and state/runtime denies; verify with unit tests `sandbox::macos::tests::paths_are_passed_as_parameters_only`, `sandbox::macos::tests::the_necessary_rules_are_always_present`, `sandbox::macos::tests::no_unix_socket_is_allowed_but_dns`
- [ ] 2.2 `executables` resolution (PATH, symlinks, Mach-O load commands with `@rpath`/`@loader_path`/`@executable_path`, `#!` interpreter); verify with unit test `sandbox::macos::tests::a_homebrew_binary_gets_its_dylibs` and `sandbox::only_granted_programs_run` on macOS
- [ ] 2.3 Spawn through `sandbox-exec` and the `tilrd sandbox-shell` trampoline with the ready pipe (D10 "Where it is applied"); start-up check and failpoint `sandbox-exec-missing`; verify with `sandbox::sandbox_is_offered_where_it_can_be_enforced` and `sandbox::a_sandbox_that_cannot_be_applied_starts_nothing` on macOS
- [ ] 2.4 Every `sandbox::*` test and `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload` green on macOS, including `sandbox::a_script_and_a_binary_made_inside_run_inside` (a script created inside runs as `./t.sh`) and `sandbox::raw_mode_and_job_control_work_in_a_sandboxed_pane`
- [ ] 2.5 CI: the sandbox tests run on every supported macOS version (matrix `macos-14`, `macos-15`, `macos-26` in the merge queue, release branches and nightly); verify the matrix job green and listed in `.github/workflows/ci.yml`

## 3. Docs

- [ ] 3.1 README "Sandboxed panes": what is granted always, what is refused, platform limits (UDP on Linux `client`, `AF_UNIX`, kernel versions; macOS: Seatbelt is deprecated by Apple but used, its profile kept in the pane's runtime directory for diagnosis); verify by review against the `pane-sandbox` spec

## 4. Checks and archive

- [ ] 4.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh` (the CI kernel must have Landlock ABI ≥ 4; note it if not); verify all green
- [ ] 4.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
