# Integration test port stand

## Purpose
Reproduce an already occupied default TCP port in a selected case's network namespace.

Verified by `itests/tests/test_port_stand.py`, including a subprocess running the real
UC-033 entry point, and paired dt runs with and without the stand.

## Requirements

### Requirement: Explicit case-local occupied port
The integration runner SHALL accept `dt --occupy-port-for MODULE:FUNCTION` for one exact
selected scenario id. Before invoking that case it SHALL bind and listen on
127.0.0.1:7890 in the case process, keep the socket open throughout the case and close
it on success or failure. It SHALL log activation. It SHALL NOT use sleeps, retries,
or increased timeouts to establish readiness, or change the case's result.

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

### Requirement: Repository fixtures for Rust tests
The dt stand SHALL include the repository fixtures needed by its enabled release
and Linux script Rust tests in both build and runtime source snapshots at the
compiled workspace path. The runtime SHALL provide Python, Git and `shasum` so
these tests run without Docker or release credentials.

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
