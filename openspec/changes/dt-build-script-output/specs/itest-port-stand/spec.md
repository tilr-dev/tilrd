## ADDED Requirements

### Requirement: Build-script output for Rust tests
The dt stand SHALL preserve the real Cargo build-script stdout files associated
with its Rust test build and make them available at the compile-time paths in
each Rust test runtime. These files SHALL belong to the exported build snapshot;
the runtime SHALL NOT require access to a shared Cargo target cache.

**Verified by:** `tilr/build_script::every_watched_file_exists_and_the_git_head_is_watched`
and `tilrd/build_script::every_watched_file_exists_and_the_git_head_is_watched`
via `./itests/dt --only rust --filter build_script`.

#### Scenario: Build-script tests run in a separate runtime
- **WHEN** dt runs the `tilr` and `tilrd` build-script tests
- **THEN** each test can read the actual Cargo stdout at its compiled `OUT_DIR/../output` path
- **AND** both tests validate the build script's watched paths without missing-file failures

#### Scenario: Cargo reuses a cached build-script result
- **WHEN** Cargo reports a cached build-script result during the Rust test build
- **THEN** dt exports that result's saved stdout into the build snapshot
- **AND** the Rust runtime reads it without mounting the build cache
