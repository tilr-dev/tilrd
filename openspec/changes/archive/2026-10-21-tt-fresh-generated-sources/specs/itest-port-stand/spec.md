## ADDED Requirements

### Requirement: Shared Cargo target source freshness
The stand SHALL serialize access to each shared Cargo target directory and make
content-changed source files, including protocol generator inputs, newer than the
cached build outputs before invoking the wrapped command. Source digest records
SHALL certify only successful commands. Before a command can partially change
artifacts, prior records SHALL be invalidated. The wrapper SHALL preserve the
command's exit status.

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
