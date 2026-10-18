## ADDED Requirements

### Requirement: The executable checked is the one exec'd
The supervisor and each pane process SHALL exec, at a reload, the very file they ran as `state-formats` and as the check child: opened once (Linux: through `/proc/self/fd`; macOS: a private copy in `<runtime dir>/exe/<pid>/`, 0700, removed by the successor). A file put at the path after it was opened MUST NOT be exec'd by that reload. Afterwards argv, `ps -o comm` and the path a later reload checks SHALL be the installed path's.

**Verified by**: `reload::the_executable_checked_is_the_one_execd`, `reload::a_pane_execs_the_executable_it_checked`, `reload::a_reload_shows_in_no_command_line_and_leaves_no_file_behind`.

#### Scenario: Replaced between the check and the exec
- **WHEN** the check passed, the supervisor sleeps before its exec (`reload-before-exec=sleep:1500`) and the executable is replaced meanwhile by a script that logs its arguments and exits 1
- **THEN** the server execs in place with the same pid and keeps its panes, the script never ran without `state-formats`, and the next reload, started by the watch, is refused with `not reloading`

#### Scenario: A pane replaced between its check and its exec
- **WHEN** a pane process sleeps between its check and its exec (`pane-reload-checked=sleep:1500`) and the executable is replaced meanwhile by that script
- **THEN** the pane process keeps its pid and its shell, and the script never ran without `state-formats`

#### Scenario: What ps shows after the exec
- **WHEN** a server with one pane reloads
- **THEN** the command lines and `ps -o comm` of the supervisor and the pane process are as before, and no `exe/` copy is left in the runtime directory

## MODIFIED Requirements

### Requirement: A second replacement during a reload costs no shell
When the executable is replaced again while a reload is under way, the server SHALL stay up with all its panes: the reload SHALL exec the file it checked, and the second file SHALL be checked by the next reload, which the watch starts.

**Verified by**: `reload::an_executable_replaced_again_during_a_reload_costs_no_shell`, `reload::the_executable_checked_is_the_one_execd`.

#### Scenario: Replaced again mid-reload
- **WHEN** the executable is replaced again while a reload sleeps 1.5 s before its exec
- **THEN** the panes survive, two panes list afterwards, and the log has no `pane process died`
