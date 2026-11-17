## MODIFIED Requirements

### Requirement: Build-script output for Rust tests
The dt stand SHALL export the Rust test build's Cargo build-script stdout and
restore it at compile-time paths in each Rust runtime, independently of the
shared Cargo target cache, at the stdout path of the Cargo version that built it.

**Verified by:** `tilr/build_script::every_watched_file_exists_and_the_git_head_is_watched`
and `tilrd/build_script::every_watched_file_exists_and_the_git_head_is_watched`
and `itests/tests/test_build.py` via `./itests/dt --only rust --filter build_script`.

#### Scenario: Build-script tests run in a separate runtime
- **WHEN** dt runs the `tilr` and `tilrd` build-script tests
- **THEN** each test can read the actual Cargo stdout at its compiled path: `OUT_DIR/../run/stdout` for a build on Cargo 1.100 or newer, `OUT_DIR/../output` on older Cargo
- **AND** both tests validate the build script's watched paths without missing-file failures

#### Scenario: Cargo reuses a cached build-script result
- **WHEN** Cargo reports a cached build-script result during the Rust test build
- **THEN** dt exports that result's saved stdout into the build snapshot
- **AND** the Rust runtime reads it without mounting the build cache

## ADDED Requirements

### Requirement: Vendored crates in the source snapshot
The dt stand SHALL include `vendor/` in its filtered source snapshot at the
compiled workspace path. That directory holds the patched crates that
`[patch.crates-io]` in `Cargo.toml` builds from, so the dt build compiles the same
dependencies as a checkout.

**Verified by:** the dt build stage (`cargo build --locked -p tilrd -p tilr`) run by
`./itests/dt --only rust --filter build_script`.

#### Scenario: The workspace patches crates.io from vendor/
- **WHEN** dt builds `tilrd` and `tilr` from the source snapshot
- **THEN** Cargo finds `libghostty-vt` and `libghostty-vt-sys` at their `vendor/` patch locations
- **AND** the build does not fail with `patch location ... does not contain packages matching`
