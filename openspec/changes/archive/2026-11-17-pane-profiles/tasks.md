# Tasks

Shared design: `openspec/design/unified-bus.md`. Numbers in brackets are the tasks of the
former single change `unified-bus` (shared design, O3).

## 1. Profiles

- [x] 1.1 [7.1] `crates/tilrd/src/profile.rs`: `profiles_dir` (P1), names, StrictModes check (P2), strict TOML, path and `[env]` rules, `[sandbox]` parsed (P3), the resolved snapshot type; verify unit tests `profile::tests::relative_paths_are_refused`, `profile::tests::tilde_is_the_home_directory`, `profile::tests::tilr_variables_cannot_be_set`, `profile::tests::every_sandbox_key_is_parsed`, `profiles::a_profile_others_can_write_is_refused`, `profiles::profiles_dir_names_another_directory`
- [x] 1.2 [7.2] `pane.create {profile}` wired into `bus/core/panes.rs` (`prepare_create` before the supervisor's part, the snapshot kept per pane for `PaneInfo.profile`/`sandboxed`), read on every create, request overrides, snapshot, errors (P4, P5, P7), `pane.profiles` (P6), sandboxed profiles refused until `sandbox-linux`; verify `profiles::a_pane_starts_with_the_profile_from_the_config_directory`, `profiles::a_profile_name_is_never_a_path`, `profiles::an_unknown_profile_is_an_error`, `profiles::a_misspelt_key_makes_the_profile_invalid`, `profiles::request_fields_override_the_profile`, `profiles::an_edited_profile_applies_to_the_next_pane_only`, `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced`, `profiles::profiles_are_listed_with_their_errors`, `pane.profiles` provided by `tilrd` in `bus.methods` (`profiles::a_pane_starts_with_the_profile_from_the_config_directory`; `bus_terminal` has no method-list test); README "Launch profiles" (new section after "The configuration file"); USE_CASES.md entries (own section)

## 2. Checks and archive

- [ ] 2.1 [12.1] Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green and every existing test passes under its name
- [x] 2.2 [12.2] Archive the change and list it under Done in `openspec/ROADMAP.md`; verify `openspec validate --all --strict`
