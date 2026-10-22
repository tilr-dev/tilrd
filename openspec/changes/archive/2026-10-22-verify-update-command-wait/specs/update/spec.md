## ADDED Requirements

### Requirement: Update command waiting is independent of launch latency
The client SHALL wait for the local update command to complete even when staged executable startup takes longer than the reload reconnect window. The command SHALL have no execution timeout; reconnect observation begins after command completion or reload.

**Verified by**: `conn::update_wait_tests::update_waits_for_command_exit_after_a_long_launch_delay`, `client::tilr_update_runs_the_update_in_the_local_server_and_waits_for_the_reload`.

#### Scenario: Staged startup outlasts the reload window
- **WHEN** a local update command has started and a day of virtual time passes before its successful exit
- **THEN** the client remains waiting, requests no execution timeout, and returns success only after receiving the command exit

#### Scenario: Integration observation follows actual staged startup
- **WHEN** the integration harness updates with real release executables whose staged version checks have not yet completed
- **THEN** it waits for the updater's installation confirmation without a launch deadline before starting the unchanged reload observation window
