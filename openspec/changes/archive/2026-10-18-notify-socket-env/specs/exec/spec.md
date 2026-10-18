## MODIFIED Requirements

### Requirement: The program runs as given, without a shell
The server SHALL start `argv[0]` (looked up in its `PATH` unless it has a `/`) with `argv[1..]` as they are: no shell, nothing expanded. It SHALL run in `cwd` (an existing directory; empty: the home directory) with the server's environment, less the service-manager variables a pane does not get either (`NOTIFY_SOCKET`, `LISTEN_FDS`, `LISTEN_FDNAMES`, `LISTEN_PID`, `WATCHDOG_USEC`, `WATCHDOG_PID`; see `panes`), plus each `env` entry `NAME=value`. Without `stdin` its stdin MUST be `/dev/null`.

**Verified by**: `exec::a_program_runs_without_a_shell_and_its_outputs_come_apart`, `exec::an_exec_does_not_get_the_service_managers_variables` and UC-077 (`uc_077.py::exec_runs_no_shell_so_nothing_is_expanded`).

#### Scenario: Nothing expanded
- **WHEN** a client runs `printf "%s|" '$HOME' '*' '`id`' 'a b'` as four separate arguments
- **THEN** stdout is `$HOME|*|`id`|a b|`

#### Scenario: Directory and environment
- **WHEN** a client runs `sh -c 'pwd -P; echo "$TT_X"'` with `cwd` an existing directory and `env` `["TT_X=from exec"]`
- **THEN** stdout is that directory then `from exec`; without `cwd` `pwd -P` prints the home directory

#### Scenario: Exec under a notify unit
- **WHEN** the server runs with `NOTIFY_SOCKET` set and an exec runs `env`, and another exec runs `env` with `env` `["NOTIFY_SOCKET=/x"]`
- **THEN** the first output has no `NOTIFY_SOCKET`, the second has `NOTIFY_SOCKET=/x`
