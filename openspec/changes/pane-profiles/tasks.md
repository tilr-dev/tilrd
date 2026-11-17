# Tasks

## 1. Profile files

- [ ] 1.1 `crates/tilrd/src/profile.rs`: name check, file location (D1, D2), strict TOML with `deny_unknown_fields`, path rules, `[env]` name rules, `[sandbox]` parsed and validated (D3); verify with unit tests `profile::tests::relative_paths_are_refused`, `profile::tests::tilde_is_the_home_directory`, `profile::tests::tilr_variables_cannot_be_set`, `profile::tests::every_sandbox_key_is_parsed`
- [ ] 1.2 Ownership and mode check on the file and the `profiles` directory (D2); verify with `profiles::a_profile_others_can_write_is_refused`; USE_CASES.md entry

## 2. Protocol and supervisor

- [ ] 2.1 `CreatePane.profile`, `PaneInfo.profile`, `ListProfiles`, `ProfileList`, `ProfileInfo` in `tilr.proto` with comments (D6); README "Protocol" describes them and says no feature flag; verify `cargo test -p tilr-proto` and review of README
- [ ] 2.2 `create_pane` reads the profile on every create and combines it with the request (D4, D5); errors as D6; verify with `profiles::a_pane_starts_with_the_profile_from_the_config_directory`, `profiles::a_profile_name_is_never_a_path`, `profiles::an_unknown_profile_is_an_error`, `profiles::a_misspelt_key_makes_the_profile_invalid`, `profiles::request_fields_override_the_profile`, `profiles::an_edited_profile_applies_to_the_next_pane_only`, the existing `panes::*` tests unchanged; USE_CASES.md entries
- [ ] 2.3 A profile with `[sandbox]` is refused (no `sandbox` feature yet); verify with `profiles::a_sandboxed_profile_is_refused_where_it_cannot_be_enforced` (marker file not created); USE_CASES.md entry
- [ ] 2.4 `ListProfiles` answered (D6, backpressure limits); verify with `profiles::profiles_are_listed_with_their_errors`; USE_CASES.md entry
- [ ] 2.5 README "Launch profiles" (new section after "The configuration file"): location, name rules, keys, example, what overrides what, that `[sandbox]` is refused until sandboxing lands; verify by review against the `pane-profiles` spec

## 3. Reload

- [ ] 3.1 `PaneState.profile` (`ProfileSnapshot`) and `SupervisorState` format 3 (D7); `tilrd state-formats` prints `supervisor-state 3`; verify with `reload::a_pane_keeps_its_profile_across_a_reload` and the existing `reload::*` format tests updated; README "Reloading without closing shells" mentions the profile; USE_CASES.md entry

## 4. tilr

- [ ] 4.1 `tilr new --profile NAME` (cwd rule of D5, the `PaneInfo.profile` check of D8); verify with `client::tilr_new_with_a_profile_starts_in_the_profiles_directory`; README "tilr" section; USE_CASES.md entry
- [ ] 4.2 `tilr profiles`; verify with `client::tilr_profiles_lists_the_profiles_of_the_server`; README; USE_CASES.md entry

## 5. Checks and archive

- [ ] 5.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo nextest run --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 --log-opts=HEAD .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 5.2 Update the row in `openspec/ROADMAP.md`, record the commit in proposal.md and archive; verify `openspec validate --all --strict`
