## MODIFIED Requirements

### Requirement: No shell outlives the server
When the server stops, or its supervisor dies, a shell (or program) in a pane that ignores SIGHUP SHALL NOT be left running as an orphan: each pane process SHALL send SIGHUP to its shell's process group and to the pty's foreground process group, SIGKILL whatever of those two groups is left 2 s later, and only then exit. Background jobs in other process groups (`nohup cmd &`) SHALL be left, as a terminal hangup leaves them.

**Verified by**: `panes::a_shell_ignoring_sighup_is_ended_when_the_server_stops`, `panes::a_foreground_program_ignoring_sighup_is_ended_when_the_server_stops`, `panes::a_nohup_background_job_is_left_when_the_server_stops`; itests `UC-086` (`uc_086.py::a_shell_ignoring_sighup_does_not_outlive_the_server`).

#### Scenario: Shell ignoring SIGHUP
- **WHEN** a pane runs a program that ignores SIGHUP and the server receives SIGTERM
- **THEN** after the server has exited, that program is no longer running

#### Scenario: Foreground job ignoring SIGHUP
- **WHEN** a pane's shell runs `sh -c 'trap "" HUP; sleep 1000'` in the foreground and the user runs `tilr kill-server`
- **THEN** when `tilr kill-server` has returned, the `sleep` is no longer running

#### Scenario: nohup job
- **WHEN** a pane's shell started `nohup sleep 1000 &` and the server receives SIGTERM
- **THEN** that `sleep` is still running after the server has exited
