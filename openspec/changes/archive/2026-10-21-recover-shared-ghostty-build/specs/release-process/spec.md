## ADDED Requirements

### Requirement: Interrupted shared Ghostty build recovery

The shared build script SHALL record its owner PID and serialize recovery with an advisory lock released on process exit. It SHALL preserve live owners, recover abandoned locks (including missing PIDs), remove abandoned temporary directories, replace incomplete prefixes before publication, and clean leftovers beside completed prefixes without rebuilding.

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

#### Scenario: Interrupted cleanup after publication
- **WHEN** a completed prefix exists alongside an abandoned lock or temporary build directory
- **THEN** the next invocation removes the abandoned state under the advisory lock and reuses the prefix without fetching or building again
