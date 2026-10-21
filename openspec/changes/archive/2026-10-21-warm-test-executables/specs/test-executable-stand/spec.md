## ADDED Requirements

### Requirement: Warm executable installations
Reload and update integration tests SHALL create private server installations by
hardlinking warmed executable inodes. The harness SHALL warm the original server
and one distinct copy with a successful `--version` invocation once per test binary.
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
