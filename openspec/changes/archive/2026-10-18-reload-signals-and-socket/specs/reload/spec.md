## ADDED Requirements

### Requirement: Signals are held across the exec
SIGUSR1, SIGHUP and SIGTERM arriving between the supervisor's exec and its successor's handlers SHALL NOT kill the supervisor: they SHALL be held across the exec and handled by the successor as the server handles them (SIGUSR1 one reload, SIGHUP ignored, SIGTERM a clean shutdown). Panes, shells and programs the successor starts SHALL NOT inherit them blocked.

**Verified by**: `reload::signals_arriving_while_the_successor_starts_are_held_for_it`, `reload::a_sigterm_while_the_successor_starts_stops_the_server_cleanly`, `reload::a_storm_of_reload_signals_costs_no_shell`.

#### Scenario: SIGHUP and SIGUSR1 right after the exec
- **WHEN** the supervisor receives SIGHUP and SIGUSR1 while its successor is starting (`supervisor-resume-live=sleep:1500`)
- **THEN** the server and its panes keep running, the log has `SIGHUP ignored`, the server reloads once more, and a shell started afterwards dies of `kill -HUP $$`

#### Scenario: SIGTERM right after the exec
- **WHEN** the supervisor receives SIGTERM while its successor is starting
- **THEN** the successor shuts down cleanly (exit status 0, `shutting down` in the log) and the shells end

#### Scenario: A storm of SIGUSR1
- **WHEN** SIGUSR1 is sent every 20 ms for 3 seconds
- **THEN** the server reloads several times, keeps its pid and its pane's shell

### Requirement: A refused reload keeps the socket serving
When a reload is refused or fails before the handover, the Unix socket file and its lock SHALL stay in place and keep serving, so that `tilr` MUST NOT start a second server.

**Verified by**: `local::a_refused_reload_keeps_the_socket_and_its_lock`; itests `UC-06B` (`uc_06b.py::a_refused_reload_keeps_the_socket_and_tilr_finds_the_same_server`).

#### Scenario: Successor fails its check
- **WHEN** a reload is refused because the successor fails the reload check
- **THEN** the socket file and its lock are in place, a local client connects through the socket, and no second `tilrd` runs

#### Scenario: A server started by tilr
- **WHEN** a server `tilr` started (with `--socket` and `--lock-fd`) has its executable replaced by a release that crashes resuming
- **THEN** the reload is refused, `tilr ls` lists the same panes, and a new `tilr` attaches to the same server with the same shell

## MODIFIED Requirements

### Requirement: A successor that cannot take the socket
If the successor does not print `unix-socket 1`, a server that serves its unix socket SHALL NOT reload into it (`not reloading: <exe> cannot take over the Unix socket this server serves`), whether or not it also has a TCP port; the socket and its lock stay served. A server without a unix socket (`--no-socket`) reloads as before.

**Verified by**: `local::a_server_on_its_socket_alone_does_not_reload_into_one_that_cannot_take_it`, `local::a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket`.

#### Scenario: Socket alone, older successor
- **WHEN** a server started with `--no-listen` has its executable replaced by one whose `state-formats` lacks `unix-socket 1`
- **THEN** the log says `cannot take over the Unix socket this server serves`, the server keeps running and `tilrd ps` works

#### Scenario: Socket and port, older successor
- **WHEN** a server with a TCP port and its unix socket has its executable replaced by one whose `state-formats` lacks `unix-socket 1`
- **THEN** it does not reload, and `tilrd ps` works through the socket
