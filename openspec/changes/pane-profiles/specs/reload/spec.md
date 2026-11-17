## MODIFIED Requirements

### Requirement: state-formats names what this executable reads
`tilrd state-formats` SHALL print one `name version` line per format it reads: `supervisor-state 3`, `pane-state 3` and `unix-socket 1` at HEAD, followed by `reload-check 1`: it answers the check of a successor.

**Verified by**: `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`; itests `UC-067` (`uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Formats of this build
- **WHEN** `tilrd state-formats` runs
- **THEN** it prints `supervisor-state 3\npane-state 3\nunix-socket 1\nreload-check 1\n`
