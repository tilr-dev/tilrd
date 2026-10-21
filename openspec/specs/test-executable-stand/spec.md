# test-executable-stand Specification

## Purpose
Starting each newly copied executable triggers a fresh macOS security assessment.
Reload and update tests can reuse warmed executable inodes while keeping private
installation paths and genuine executable replacement.

## Requirements

### Requirement: Warm executable installations
Reload and update integration tests SHALL create private server installations by
hardlinking warmed executable inodes. The harness SHALL warm the original server and a reusable pair of inodes with
successful `--version` invocations once per test binary. The pair SHALL reuse the
original inode when it is on the tests' filesystem, or use a once-created local
copy when Cargo's output is on a separate filesystem.
Replacement helpers SHALL select an inode different from the current executable,
including on consecutive replacements, so rename cannot become a no-op.

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
