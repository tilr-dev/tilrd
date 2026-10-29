## MODIFIED Requirements

### Requirement: Warm executable installations
Reload and update tests SHALL hardlink private installations to two reusable
inodes. Once per test binary, the harness SHALL warm the original and each new inode
with successful `--version`, reusing the original inode on the same filesystem or copying
it once locally otherwise. Replacements SHALL alternate inodes so rename consumes the
staged path. Warm pairs SHALL use `$TMPDIR/tilr-warm-<pid>/`; at startup the
harness SHALL remove exited processes' pairs and preserve live processes' pairs.

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

#### Scenario: Exited test processes leave no accumulated pairs
- **WHEN** a new test binary initializes its warmed executable pair
- **THEN** it removes `tilr-warm-<pid>` directories of exited processes
- **AND** it preserves directories of live processes and uses its own process directory

**Verified by**: `reload::warm_executables_do_not_accumulate_between_processes`.
