# Tasks

## 1. Script tests

- [x] 1.1 Harness in `crates/tilrd/tests/release.rs`: a temp git repo with a copy of `scripts/release.sh`, `Cargo.toml`, licenses and README, a stand-in `scripts/linux-build.sh` and a stand-in `wrangler` logging to a file; verify `release::a_dirty_tree_builds_no_release` (exit 1, message, nothing in `dist/cdn`) and `release::a_release_is_named_after_the_package_version_and_the_commit`
- [x] 1.2 Platforms; verify `release::each_platform_given_is_built_with_its_own_latest` and `release::an_unknown_platform_is_refused` (exit 2)
- [x] 1.3 Uploads; verify `release::publishing_without_the_account_id_does_nothing` (exit 2, no build, empty wrangler log), `release::publishing_uploads_the_archive_and_its_checksum_before_latest` (wrangler log order, content types, cache control, `--remote`) and `release::upload_only_checks_what_was_built_and_leaves_other_platforms_alone` (missing `latest`, mixed versions, bad checksum each refused; other platform's `latest` not uploaded)
- [x] 1.4 Layout contract; verify `release::the_layout_release_sh_writes_is_what_tilrd_update_reads` (`dist/cdn` served on a loopback port as `update.rs`'s `serve_files` does; `tilrd update` finds the version and unpacks `tilrd`)
- [x] 1.5 USE_CASES.md entries for 1.1-1.4; verify by review

## 2. Static tests

- [x] 2.1 `release::the_linux_build_runs_in_the_pinned_debian_12_image`, `release::build_tools_are_pinned_by_checksum_and_used_everywhere`; verify green
- [x] 2.2 `release::ci_runs_every_gate_of_agents_md`, `release::ci_smoke_tests_both_executables`; verify green, and red when a gate line is removed from a copy of `ci.yml` (checked once by hand)
- [x] 2.3 `release::release_please_bumps_the_workspace_version_and_lock`, `release::a_release_builds_and_packages_every_target`, `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built`; verify green; USE_CASES.md entries

## 3. Checks and archive

- [x] 3.1 Before marking a change done, run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also the Linux run via `scripts/linux.sh`; verify all green
- [ ] 3.2 Refresh the MODIFIED requirements from the current `openspec/specs/release-process/spec.md` if another change touched them; drop "(planned …)"; shrink the spec's Notes; verify `openspec validate release-process-tests --strict`
- [ ] 3.3 No itests case is touched; `./itests/dt coverage` exits 0 with no `release-process` requirement uncovered
- [ ] 3.4 Record the implementing commit in proposal.md and archive the change; verify `openspec list` no longer shows it
