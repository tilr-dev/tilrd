## MODIFIED Requirements

### Requirement: The successor must run and read the state formats
Before handing over, the supervisor SHALL run the executable on disk as `tilrd state-formats` (10 s limit) and SHALL go on with the reload only if it exits 0, prints `supervisor-state` and `pane-state` versions equal to the ones this server writes (2 and 3 at HEAD; 0 counts as 1) and prints `reload-check 1`. Else it SHALL log `not reloading: ...` and carry on with its code and every connection; so no release before the reload check is reloaded into.

**Verified by**: `reload::a_broken_executable_is_not_reloaded_into`, `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`, unit test `reload::tests::check_formats_rejects_every_format_mismatch` (crates/tilrd/src/reload.rs); itests `UC-067` (`uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Executable that does not run
- **WHEN** the executable is replaced by a script that exits 1
- **THEN** the server logs `not reloading`, stays up, and the same client runs a command in its pane

#### Scenario: Another state format
- **WHEN** the new executable prints `supervisor-state 2` and `pane-state 2`
- **THEN** the server logs `reads pane-state format 2, this server writes 3` and does not reload; a later executable printing the matching formats is reloaded into

#### Scenario: An executable without the reload check
- **WHEN** the new executable prints this server's formats but no `reload-check 1`
- **THEN** the server logs `not reloading: ... cannot show that it can resume` and carries on
