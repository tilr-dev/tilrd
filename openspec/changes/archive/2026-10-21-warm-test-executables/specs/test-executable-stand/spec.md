## ADDED Requirements

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
