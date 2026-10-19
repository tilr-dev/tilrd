## MODIFIED Requirements

### Requirement: Stale state files are swept at start
A supervisor that starts or has just reloaded SHALL remove `server-*.state` in the runtime directory and `*.state` in `panes/`, and their `.*.state.tmp` temporaries, that are older than one hour.

**Verified by**: `reload::stale_state_files_are_swept_at_start` (planned in change server-requirement-tests).

#### Scenario: Leftover from a crashed reload
- **WHEN** a `server-1234.state` file two hours old is in the runtime directory and the server starts
- **THEN** the file is removed
