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
