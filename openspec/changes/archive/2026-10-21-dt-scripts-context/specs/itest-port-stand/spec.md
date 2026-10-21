## ADDED Requirements

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
