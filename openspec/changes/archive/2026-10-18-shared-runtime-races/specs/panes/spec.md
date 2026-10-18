## ADDED Requirements

### Requirement: The shell wrappers survive another server starting
The wrappers in `<runtime dir>/shell/` are shared by all servers of the user. A starting server SHALL write a wrapper only when its content differs, whole (through a temporary file of its own, renamed over it), and two servers starting at once MUST both succeed, so neither removes the wrappers the other's panes start through.

**Verified by**: unit test `shell::tests::servers_starting_at_once_each_install_the_wrappers` (crates/tilrd/src/shell.rs).

#### Scenario: Servers starting at once
- **WHEN** four installs of the wrappers run at once, 100 times each, with an older copy put back before each
- **THEN** every install succeeds and the directory holds the current wrapper and nothing beside it
