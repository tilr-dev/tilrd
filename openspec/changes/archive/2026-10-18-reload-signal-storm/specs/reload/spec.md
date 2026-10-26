## MODIFIED Requirements

### Requirement: Signals are held across the exec
SIGUSR1, SIGHUP, SIGTERM and SIGINT arriving between the moment the supervisor commits to its exec and its successor's handlers SHALL NOT kill the supervisor and SHALL NOT be taken by any thread of the image being replaced: they SHALL be held across the exec and handled by the successor as the server handles them (SIGUSR1 one reload, SIGHUP ignored, SIGTERM and SIGINT a clean shutdown). Panes, shells and programs the supervisor or its successor starts SHALL NOT inherit them blocked.

**Verified by**: `reload::a_sigterm_while_the_supervisor_execs_waits_for_the_successor`, `reload::signals_arriving_while_the_successor_starts_are_held_for_it`, `reload::a_sigterm_while_the_successor_starts_stops_the_server_cleanly`, `reload::a_sigint_while_the_successor_starts_stops_the_server_cleanly`, `reload::a_storm_of_reload_signals_costs_no_shell`.

#### Scenario: SIGTERM while the supervisor execs
- **WHEN** the supervisor receives SIGTERM right before its exec (`reload-exec=sleep:1500`)
- **THEN** no thread of the old image takes it: the successor logs `SIGTERM came during the reload`, shuts down cleanly (exit status 0, `shutting down` in the log) and the shells end

#### Scenario: SIGHUP and SIGUSR1 right after the exec
- **WHEN** the supervisor receives SIGHUP and SIGUSR1 while its successor is starting (`supervisor-resume-live=sleep:1500`)
- **THEN** the server and its panes keep running, the log has `SIGHUP ignored`, the server reloads once more, and a shell started afterwards dies of `kill -HUP $$`

#### Scenario: SIGTERM right after the exec
- **WHEN** the supervisor receives SIGTERM while its successor is starting
- **THEN** the successor shuts down cleanly (exit status 0, `shutting down` in the log) and the shells end

#### Scenario: SIGINT right after the exec
- **WHEN** the supervisor receives SIGINT while its successor is starting (`supervisor-resume-live=sleep:1500`)
- **THEN** the successor shuts down cleanly (exit status 0, `shutting down` in the log), the shells end and the socket and lock file are removed

#### Scenario: A storm of SIGUSR1
- **WHEN** SIGUSR1 is sent every 20 ms for 3 seconds
- **THEN** the server reloads several times, keeps its pid and its pane's shell
