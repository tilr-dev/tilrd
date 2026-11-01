## ADDED Requirements

### Requirement: Complete release regression runtime dependencies
The dt stand SHALL include `.github/workflows/release-upstream.yml`,
`.github/workflows/spec-change.yml` and `.config/nextest.toml` in its filtered
source snapshot. Rust test runtimes SHALL provide the compiler from the pinned
Rust build image and a native linker so release-key validation runs in dt.

**Verified by:** `tilrd/release::release_upstream_workflow_is_trusted`,
`tilrd/release::the_spec_change_check_compares_main_with_the_test_merge`,
`tilrd/release::the_quick_profile_skips_only_existing_tests` and
`tilrd/build_script::release_builds_cannot_trust_the_debug_fixture_key` via dt.

#### Scenario: Release configuration is available in the snapshot
- **WHEN** dt runs the Rust release regression tests
- **THEN** they can read the upstream-release and spec-change workflows and nextest configuration at their compile-time repository paths

#### Scenario: Release-key validation compiles its build script
- **WHEN** dt runs the release-key build-script regression
- **THEN** rustc compiles the build script and links an executable in the Rust runtime
- **AND** the test checks release and debug key acceptance without skipping validation
