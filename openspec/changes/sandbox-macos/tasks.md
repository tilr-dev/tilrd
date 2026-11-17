# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former `unified-bus` (its `tasks.md` at 75fc8ce0).

## 1. macOS sandbox

- [ ] 1.1 [8.5] macOS profile generation (SBPL with fixed rules, `(param …)` only, NUL/newline refused, the always-allowed set, `read_only`/`read_write`/`network`, Unix-socket and state/runtime/config denies; S10); verify unit tests `sandbox::macos::tests::paths_are_passed_as_parameters_only`, `sandbox::macos::tests::the_necessary_rules_are_always_present`, `sandbox::macos::tests::no_unix_socket_is_allowed_but_dns`
- [ ] 1.2 [8.6] macOS `executables` resolution (PATH, symlinks, Mach-O load commands with `@rpath`/`@loader_path`/`@executable_path`, `#!` interpreter); verify unit test `sandbox::macos::tests::a_homebrew_binary_gets_its_dylibs` and `sandbox::only_granted_programs_run` on macOS
- [ ] 1.3 [8.7] macOS spawn through `sandbox-exec` and the `tilrd sandbox-shell` trampoline with the ready pipe, start-up check, failpoint `sandbox-exec-missing`; verify every `sandbox::*` test green on macOS (and `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload` once `bus-reload` landed); USE_CASES.md entries (own section)
- [ ] 1.4 [8.8] CI job `sandbox-macos` (matrix `macos-14`, `macos-15`, `macos-26`) in the merge queue, release branches and nightly; verify the job green and listed in `.github/workflows/ci.yml`
- [ ] 1.5 [8.9, macOS] README "Sandboxed panes", macOS part: Seatbelt deprecated but used, its profile kept for diagnosis, what is always allowed; verify by review against the `pane-sandbox` spec

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [ ] 2.2 [12.2] Update this change's row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
