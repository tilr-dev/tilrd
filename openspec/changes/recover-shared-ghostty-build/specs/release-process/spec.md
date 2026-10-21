## ADDED Requirements

### Requirement: Interrupted shared Ghostty build recovery

The shared build script SHALL record its owner PID in the prefix lock. When that process is no longer alive, a subsequent invocation SHALL recover the lock, remove abandoned temporary build directories for that prefix, and build again. It SHALL refuse to disturb a lock owned by a live process. Before publishing a completed installation, it SHALL remove an incomplete destination prefix.

Recovery SHALL be serialized by an advisory lock that is released on process exit, including an untrappable termination. A lock directory left before its owner PID was recorded SHALL also be recoverable.

#### Scenario: Recover an abandoned build
- **WHEN** a lock records the PID of an exited process and temporary build directories remain
- **THEN** the next invocation builds and publishes the completed prefix
- **AND** no lock or temporary build directories remain

#### Scenario: Preserve an active build
- **WHEN** the lock records a live process PID
- **THEN** the invocation reports that a build is in progress without removing its lock or temporary directories

#### Scenario: Replace an incomplete prefix
- **WHEN** the destination exists without a completion marker
- **THEN** a successful build replaces it with the library and pkg-config file at the expected paths

#### Scenario: Interrupted owner initialization
- **WHEN** a previous build left a lock directory without a PID and no process holds the advisory lock
- **THEN** the next invocation recovers the directory and publishes a completed prefix

#### Scenario: Concurrent recovery
- **WHEN** one invocation holds the advisory lock while recovering an abandoned build
- **THEN** another invocation reports a build in progress without removing the first invocation's lock or working directory, even if the owner metadata is stale
