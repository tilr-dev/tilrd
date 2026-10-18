## ADDED Requirements

### Requirement: The successor shows it can resume before the exec
Before its exec, and before it stops anything for the reload, the supervisor SHALL run the executable on disk with the server's command line and environment and `TILR_RELOAD_CHECK` naming a copy of the state it would hand over (`server-<pid>.check.state`, 0600, in the runtime directory). It SHALL exec only after that process answers `ok` on its stdout and exits 0 within 10 s, and SHALL serve its clients and panes while it waits.

**Verified by**: `reload::a_successor_that_panics_resuming_is_not_reloaded_into`, `reload::a_successor_is_checked_while_the_server_serves_and_then_reloaded_into`; itests `UC-06A` (`uc_06a.py::a_version_that_crashes_resuming_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Successor panics on resume
- **WHEN** the failpoint `supervisor-resume=panic` is set and the server gets `SIGUSR1`
- **THEN** the log has `not reloading: the new executable could not resume` and `failpoint supervisor-resume`, the server keeps its pid, the same client runs a command in its pane with the same shell pid, and no `*.state` file is left

#### Scenario: A slow check that passes
- **WHEN** the check takes 1.5 s (`supervisor-resume=sleep:1500`) and then answers `ok`
- **THEN** the same client runs a command during the check, and afterwards the server execs in place with the same pid and keeps its pane and shell

#### Scenario: A release that crashes resuming
- **WHEN** `tilrd update` installs a release whose `tilrd` reads this server's state but crashes as it resumes
- **THEN** the server does not exec it, keeps its pid, listener, connections and every program in its panes, and still reports the old version

### Requirement: A successor that fails its check is not reloaded into
When the check process answers `error: <reason>`, ends in any other way or does not answer within 10 s, the supervisor SHALL kill it if it still runs, remove the check file, log `not reloading: the new executable could not resume: <reason>` (its answer, or how it ended and the last lines of its stderr) and carry on unchanged: same pid, connections, execs, panes and shells.

**Verified by**: `reload::a_successor_that_rejects_the_command_line_is_not_reloaded_into`, `reload::a_successor_that_refuses_the_configuration_is_not_reloaded_into`, `reload::a_successor_that_never_answers_the_check_is_not_reloaded_into`.

#### Scenario: Successor rejects the command line
- **WHEN** a server started with `--socket` gets `SIGUSR1` after its executable was replaced by one that prints this server's formats in `state-formats` but exits 2 with `error: unexpected argument '--socket' found` when given `--socket`
- **THEN** no exec happens, the server keeps its pid, connection and panes, its socket stays, and the log names `--socket`

#### Scenario: Configuration the successor refuses
- **WHEN** a key the executable does not know is added to the configuration file and the server gets `SIGUSR1`
- **THEN** no exec happens, the log has `unknown field`, and the server keeps running with its panes; with the key removed the next reload goes through

#### Scenario: Successor that never answers
- **WHEN** the check process neither answers nor exits within 10 s (`supervisor-resume=sleep:20000`)
- **THEN** the same client runs commands while it waits, the process is killed after 10 s, the log says `did not answer within 10 s`, the reload is refused, and no `*.check.state` file is left

### Requirement: The check process resumes without taking over
A `tilrd` started with `TILR_RELOAD_CHECK` SHALL parse its command line, load the configuration file, read the state file the variable names and resume up to taking over the descriptors it names (a pane process restores its screen), then write one line, `ok` or `error: <reason>` (a panic included), to stdout and exit 0 or 1. It MUST NOT bind, lock, accept, start panes or `cloudflared`, or touch those descriptors.

**Verified by**: `reload::a_successor_that_refuses_the_configuration_is_not_reloaded_into`, `reload::a_pane_whose_successor_fails_the_check_carries_on`.

#### Scenario: Check of a pane process
- **WHEN** a pane process's successor is run with `TILR_RELOAD_CHECK` and its resume panics (`pane-resume=panic`)
- **THEN** it answers `error: panicked ...`, and the pane process it checked for keeps its pty and shell

## MODIFIED Requirements

### Requirement: The successor must run and read the state formats
Before handing over, the supervisor SHALL run the executable on disk as `tilrd state-formats` (10 s limit) and SHALL go on with the reload only if it exits 0, prints `supervisor-state` and `pane-state` versions equal to the ones this server writes (2 and 3 at HEAD; 0 counts as 1) and prints `reload-check 1`. Else it SHALL log `not reloading: ...` and carry on with its code and every connection; so no release before the reload check is reloaded into.

**Verified by**: `reload::a_broken_executable_is_not_reloaded_into`, `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`; itests `UC-067` (`uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Executable that does not run
- **WHEN** the executable is replaced by a script that exits 1
- **THEN** the server logs `not reloading`, stays up, and the same client runs a command in its pane

#### Scenario: Another state format
- **WHEN** the new executable prints `supervisor-state 2` and `pane-state 2`
- **THEN** the server logs `reads pane-state format 2, this server writes 3` and does not reload; a later executable printing the matching formats is reloaded into

#### Scenario: An executable without the reload check
- **WHEN** the new executable prints this server's formats but no `reload-check 1`
- **THEN** the server logs `not reloading: ... cannot show that it can resume` and carries on

### Requirement: state-formats names what this executable reads
`tilrd state-formats` SHALL print one `name version` line per format it reads: `supervisor-state 2`, `pane-state 3` and `unix-socket 1` at HEAD, followed by `reload-check 1`: it answers the check of a successor.

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
Before its own exec a pane process SHALL check its successor the same way, with a copy of its state (`panes/<pane id>.check.state`). A pane process whose reload fails before its exec, its check included, SHALL log `reload failed` (for a failed check with `the new executable could not resume` and the reason), remove its state files and carry on in the code it has with its shell; a pane process that crashes in its live resume SHALL be lost (its shell ends) and the other panes SHALL carry on.

**Verified by**: `reload::a_pane_that_cannot_hand_over_carries_on_in_the_code_it_has`, `reload::a_pane_whose_successor_fails_the_check_carries_on`, `reload::a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on`, `reload::a_shell_that_exits_after_a_reload_still_reports_its_exit_code`.

#### Scenario: Pane resume panics in the check
- **WHEN** the failpoint `pane-resume=panic` is set and the server reloads
- **THEN** the supervisor runs the new code, the pane process logs `reload failed` and `the new executable could not resume`, and the pane is still listed with the same shell, its variables and its screen

#### Scenario: One pane crashes taking over
- **WHEN** two panes are open and the first pane's process panics in its live resume after a reload (`pane-resume-live:<pane id>=panic`)
- **THEN** the server logs `pane process died`, lists only the second pane with its same shell and variables, and the first pane's shell exits
