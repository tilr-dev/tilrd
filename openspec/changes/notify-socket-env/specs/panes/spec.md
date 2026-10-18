## MODIFIED Requirements

### Requirement: User variables of a pane
`HOME`, `USER` and `LOGNAME` SHALL come from the password database when the server's own value is unset or empty, and pass through otherwise; `XPC_SERVICE_NAME`, `XPC_FLAGS` and the service-manager variables `NOTIFY_SOCKET`, `LISTEN_FDS`, `LISTEN_FDNAMES`, `LISTEN_PID`, `WATCHDOG_USEC` and `WATCHDOG_PID` SHALL be removed, even when the server was started with them. When the server has tools for the panes it SHALL put their directory first on `PATH` and name it in `TILR_SHIMS` (see `clipboard-host-events`).

**Verified by**: `panes::a_pane_without_a_command_runs_the_users_login_shell` and `panes::a_pane_does_not_get_the_service_managers_variables`.

#### Scenario: Who and where
- **WHEN** `test "$HOME" = <home> && test -n "$USER" && test "$LOGNAME" = "$USER"` runs in a login-shell pane
- **THEN** it exits 0

#### Scenario: The tools survive the login profile
- **WHEN** the login profile has run in a new pane
- **THEN** `$TILR_SHIMS` is not empty and is one of the directories on `$PATH`

#### Scenario: Server under a notify unit
- **WHEN** the server runs with `NOTIFY_SOCKET`, `LISTEN_FDS` and `WATCHDOG_USEC` set and a pane runs `env`
- **THEN** none of them is in the output
