## MODIFIED Requirements

### Requirement: Explicit case-local occupied port
The integration runner SHALL accept `dt --occupy-port-for MODULE:FUNCTION` for one exact
selected scenario id. Before invoking that case it SHALL bind and listen on
127.0.0.1:7890 in the case process, keep the socket open throughout the case and close
it on success or failure. It SHALL log activation. It SHALL NOT use sleeps, retries,
or increased timeouts to establish readiness, or change the case's result.

**Verified by**: `itests/tests/test_port_stand.py::PortStandTest`.

#### Scenario: Reproduce uc_033's bind failure
- **WHEN** the stand targets `uc_033:a_tilrd_tilr_starts_goes_on_without_the_taken_port`
- **THEN** its initial bind to 7890 fails with EADDRINUSE and the report remains FAIL
- **AND** dt exits nonzero

#### Scenario: Normal and other cases
- **WHEN** the option is omitted, or another case runs alongside the target
- **THEN** the runner creates no listener for that case

#### Scenario: Invalid selection
- **WHEN** the target is unknown, excluded by the filter, or scenarios are excluded
- **THEN** the runner rejects the option before running tests

#### Scenario: Setup cannot bind
- **WHEN** 7890 is already occupied before stand setup
- **THEN** setup fails visibly rather than treating an unrelated listener as its holder

### Requirement: Shared Cargo target source freshness
The stand SHALL serialize access to each shared Cargo target directory and make
content-changed source files, including protocol generator inputs, newer than the
cached build outputs before invoking the wrapped command. Source digest records
SHALL certify only successful commands. Before a command can partially change
artifacts, prior records SHALL be invalidated. The wrapper SHALL preserve the
command's exit status.

**Verified by**: `itests/tests/test_fresh.py::FreshTest`.

#### Scenario: Failed build before the next target is built
- **GIVEN** both target directories contain artifacts from an older protocol
- **WHEN** a changed protocol is presented with old source mtimes and the command fails before rebuilding the next target
- **AND** the same changed source tree is mounted again with old mtimes
- **THEN** the next invocation refreshes the source mtimes and regenerates the next target's protocol code

#### Scenario: Successful unchanged build
- **WHEN** the same content is presented after a successful build
- **THEN** the wrapper retains the successful digests and does not touch unchanged files

#### Scenario: Missing source digest record
- **WHEN** a target has no successful source digest record
- **THEN** all current source files are refreshed before the command runs

### Requirement: Repository fixtures for Rust tests
The dt stand SHALL include the repository fixtures needed by its enabled release
and Linux script Rust tests in both build and runtime source snapshots at the
compiled workspace path. The runtime SHALL provide Python, Git and `shasum` so
these tests run without Docker or release credentials.

**Verified by**: `release::publishing_uploads_the_archive_and_its_checksum_before_latest` and `linux_script::parallel_worktrees_keep_their_linux_build_outputs` via `./itests/dt --only rust --filter release` and `./itests/dt --only rust --filter linux_script`.

#### Scenario: Release fixtures run in dt
- **WHEN** dt runs the Rust `release` suite
- **THEN** the tests can read their scripts and repository configuration
- **AND** the snapshot includes release workflows, release-please configuration,
  tracked Cargo configuration and Git ignore rules
- **AND** the release fixtures can create and verify archive checksums

#### Scenario: Linux worktree fixture runs in dt
- **WHEN** dt runs `linux_script::parallel_worktrees_keep_their_linux_build_outputs`
- **THEN** the Python regression fixture and its shell scripts are present
- **AND** its controlled Docker substitute verifies isolated worktree build outputs

### Requirement: Build-script output for Rust tests
The dt stand SHALL export the Rust test build's Cargo build-script stdout and
restore it at compile-time paths in each Rust runtime, independently of the
shared Cargo target cache.

**Verified by:** `tilr/build_script::every_watched_file_exists_and_the_git_head_is_watched`
and `tilrd/build_script::every_watched_file_exists_and_the_git_head_is_watched`
and `itests/tests/test_build.py::BuildTest::test_build_script_stdout_survives_without_the_build_cache` via `./itests/dt --only rust --filter build_script`.

#### Scenario: Build-script tests run in a separate runtime
- **WHEN** dt runs the `tilr` and `tilrd` build-script tests
- **THEN** each test can read the actual Cargo stdout at its compiled `OUT_DIR/../output` path
- **AND** both tests validate the build script's watched paths without missing-file failures

#### Scenario: Cargo reuses a cached build-script result
- **WHEN** Cargo reports a cached build-script result during the Rust test build
- **THEN** dt exports that result's saved stdout into the build snapshot
- **AND** the Rust runtime reads it without mounting the build cache
