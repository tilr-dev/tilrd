## MODIFIED Requirements

### Requirement: No shell outlives the server
When the server stops or its supervisor dies, a program in a pane that ignores SIGHUP SHALL NOT be left running: each pane process SHALL send SIGHUP to its shell's process group and the pty's foreground process group, SIGKILL what is left of them 2 s later, then exit; a stopping server SHALL wait up to 3 s for its pane processes. Background jobs in other groups (`nohup cmd &`) SHALL be left.

**Verified by**: `panes::a_shell_ignoring_sighup_is_ended_when_the_server_stops`, `panes::a_foreground_program_ignoring_sighup_is_ended_when_the_server_stops`, `panes::a_nohup_background_job_is_left_when_the_server_stops`; itests `UC-086` (`uc_086.py::a_shell_ignoring_sighup_does_not_outlive_the_server`).

#### Scenario: Shell ignoring SIGHUP
- **WHEN** a pane runs a program that ignores SIGHUP and the server receives SIGTERM
- **THEN** once the server and its pane process have exited, that program is no longer running

#### Scenario: Foreground job ignoring SIGHUP
- **WHEN** a pane's shell runs `sh -c 'trap "" HUP; sleep 1000'` in the foreground and the user runs `tilr kill-server`
- **THEN** when `tilr kill-server` has returned, the `sleep` is no longer running

#### Scenario: nohup job
- **WHEN** a pane's shell started `nohup sleep 1000 &` and the server receives SIGTERM
- **THEN** that `sleep` is still running after the server and its pane process have exited
