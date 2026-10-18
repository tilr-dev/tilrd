## ADDED Requirements

### Requirement: The successor shows it can resume before the exec
Before its exec, when the executable on disk prints `reload-check 1` in `tilrd state-formats`, the supervisor SHALL run it with the server's command line, `TILR_RELOAD_CHECK` set and a copy of the state (`server-<pid>.check.state`, 0600), and SHALL exec only after it answers `ok` and exits 0 within 10 s. Otherwise it SHALL log `not reloading: the new executable could not resume: <reason>`, remove the check and state files and carry on unchanged.

#### Scenario: Successor panics on resume
- **WHEN** the failpoint `supervisor-resume=panic` is set and the server gets `SIGUSR1`
- **THEN** the log has `not reloading: the new executable could not resume`, the server keeps its pid, and the same client runs a command in its pane with the same shell pid

#### Scenario: Successor rejects the command line
- **WHEN** a server started with `--socket` and `--lock-fd` gets `SIGUSR1` after its executable was replaced by one that prints `reload-check 1` but does not accept `--socket`
- **THEN** no exec happens, the server keeps its pid and panes, and the log names the rejected argument

#### Scenario: Configuration the successor refuses
- **WHEN** the configuration file has a key the executable on disk rejects and the server gets `SIGUSR1`
- **THEN** no exec happens and the server keeps running with its panes

#### Scenario: Successor that never answers
- **WHEN** the check process neither answers nor exits within 10 s
- **THEN** it is killed, the reload is refused as above, and no `*.check.state` file is left

### Requirement: The check process resumes without taking over
A `tilrd` started with `TILR_RELOAD_CHECK` SHALL parse its command line, load the configuration file and read the state as a live resume would, then answer `ok` or `error: <reason>` and exit; it MUST NOT bind, lock, accept, start panes or start `cloudflared`.

#### Scenario: The server is untouched by a check
- **WHEN** the supervisor runs a passing check of its successor and then execs
- **THEN** no second listener, socket or lock appeared during the check and `tilrd ps` lists the same panes

### Requirement: An older successor is checked by its flags
A successor whose `state-formats` lacks `reload-check 1` SHALL be reloaded into without the check only when every flag in the server's command line is one that releases before `reload-check` accept; otherwise the supervisor SHALL log `not reloading: <exe> does not accept <flag>` and carry on.

#### Scenario: Rollback to a release without --socket
- **WHEN** a server started by `tilr` with `--socket` and `--lock-fd` gets `SIGUSR1` after its executable was replaced by a release that does not know those flags and prints no `reload-check` line
- **THEN** no exec happens, the server keeps its pid and panes, and the log names `--socket`

### Requirement: A pane process checks its successor
Before its own exec, when the successor prints `reload-check 1`, a pane process SHALL run the same check with a copy of its pane state (`panes/<pane id>.check.state`); when the check fails it SHALL log `reload failed` with the reason, remove its state files and carry on in the code it has with its shell.

#### Scenario: Pane resume panics in the check
- **WHEN** the failpoint `pane-resume=panic` is set and the server reloads
- **THEN** the supervisor runs the new code, the pane is still listed with the same shell pid, and `test "$KEPT" = yes` exits 0 in it

## MODIFIED Requirements

### Requirement: state-formats names what this executable reads
`tilrd state-formats` SHALL print one `name version` line per format it reads: `supervisor-state 2`, `pane-state 3` and `unix-socket 1` at HEAD, followed by `reload-check 1` from an executable that can run the reload check.

**Verified by**: `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`; itests `UC-067` (`uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Formats of this build
- **WHEN** `tilrd state-formats` runs
- **THEN** it prints `supervisor-state 2\npane-state 3\nunix-socket 1\nreload-check 1\n`

### Requirement: No shell outlives a server lost in a reload
A successor that passed the reload check and then fails or crashes taking over SHALL end the server, as SHALL a supervisor killed mid-reload; every shell SHALL then exit, so none is left where no client can reach it. `SIGTERM` before or after a reload SHALL end the shells.

**Verified by**: `reload::a_version_that_crashes_taking_over_ends_the_server_and_its_shells`, `reload::a_server_killed_in_the_middle_of_a_reload_leaves_no_shell_behind`, `reload::stopping_the_server_after_a_reload_ends_the_shells`.

#### Scenario: Successor panics on resume
- **WHEN** the failpoint `supervisor-resume-live=panic` is set and the server reloads
- **THEN** the server process ends and the pane's shell exits

### Requirement: A pane that cannot hand over carries on
A pane process whose reload fails before its exec (its check included) SHALL log `reload failed` and carry on in the code it has with its shell; a pane process that crashes in its live resume SHALL be lost (its shell ends) and the other panes SHALL carry on.

**Verified by**: `reload::a_pane_that_cannot_hand_over_carries_on_in_the_code_it_has`, `reload::a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on`, `reload::a_shell_that_exits_after_a_reload_still_reports_its_exit_code`.

#### Scenario: One pane crashes taking over
- **WHEN** two panes are open and the first pane's process panics in its live resume after a reload (`pane-resume-live:<pane id>=panic`)
- **THEN** the server logs `pane process died`, lists only the second pane with its same shell and variables, and the first pane's shell exits
