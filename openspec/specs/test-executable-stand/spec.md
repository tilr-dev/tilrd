# test-executable-stand Specification

## Purpose
Starting each newly copied executable triggers a fresh macOS security assessment.
Reload and update tests can reuse warmed executable inodes while keeping private
installation paths and genuine executable replacement.

## Requirements

### Requirement: Warm executable installations
Reload and update tests SHALL hardlink private installations to two reusable
inodes. Once per test binary, the harness SHALL warm the original and each new
inode with a successful `--version`. It SHALL reuse the original inode on the
same filesystem, or copy it once into the tests' filesystem otherwise.
Replacements SHALL alternate inodes so rename consumes the staged path.

**Verified by**: `reload::warm_executable_links_reuse_inodes_and_replace_repeatedly`.

#### Scenario: Multiple private installations
- **WHEN** tests create private server installations
- **THEN** they share warmed executable inodes without copying the server per test

#### Scenario: Consecutive replacements
- **WHEN** a test replaces its executable twice
- **THEN** each staged file has a different inode from the current executable
- **AND** each rename consumes the staged path and changes the executable inode

#### Scenario: Cargo output on another filesystem
- **WHEN** Cargo's output directory is on a separate volume from test installations
- **THEN** the harness creates and warms two local executable copies once
- **AND** all private installations and replacements use hardlinks to that pair

### Requirement: Compatibility probes distinguish launch admission from execution
With `failpoints`, explicitly opted-in compatibility probes SHALL wait for a
successor startup event before starting the ten-second execution deadline.
Execution, stdout collection, and exit SHALL remain bounded by that deadline.
Ordinary probes and builds without `failpoints` SHALL retain the ten-second
limit including startup. The successor resume-check deadline SHALL remain
unchanged.

**Verified by**: `reload::tests::delayed_state_formats_obeys_the_selected_policy`.

#### Scenario: Launch admission exceeds the execution deadline
- **WHEN** a test holds a successor before its startup event past ten seconds
- **THEN** the opted-in probe remains pending
- **AND** after startup and release it reads the successor formats successfully
- **AND** the same held successor without the opt-in times out

#### Scenario: Successor stalls after startup
- **WHEN** the opted-in successor signals startup and then stops making progress
- **THEN** the probe times out after the existing ten-second execution deadline
