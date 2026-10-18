# daemon-lifecycle Specification

## Purpose

Defines tilrd as a long-running daemon: what it listens on (TCP port and Unix socket), where its runtime and state directories are and their modes, how `tilr` starts it detached and makes sure one server serves a socket, what it does with a socket left at its path, how it treats SIGHUP, SIGTERM and SIGINT, and what stopping leaves behind. Who may connect (token, origins, peer uid) is the `access-control` spec; reloading (SIGUSR1, the executable changing, handing the sockets over) is the `reload` spec; the `tilr` commands themselves are the `tilr-cli` spec; how a pane closes its shell is the `panes` spec.

## Requirements

### Requirement: Foreground service with its log on stderr
`tilrd` with no subcommand SHALL run in the foreground until stopped and SHALL write its log to stderr, with colors only when stderr is a terminal and `NO_COLOR` is unset or empty. Once it listens it SHALL log `listening on ws://<addr>` for the TCP port and `Unix socket for local clients` with the socket's path. An error that stops it at start SHALL end it with exit status 1 and the error on stderr; a stop on SIGTERM SHALL exit 0.

**Verified by**: itests `UC-016` (`uc_016.py::tilr_attaches_to_a_tilrd_running_in_the_foreground`) and `local::a_server_that_stops_removes_its_socket_and_lock`.

#### Scenario: Run by hand
- **WHEN** the user runs `tilrd` with a clean environment and then `tilr`
- **THEN** stderr says `Unix socket for local clients` and `listening on ws://127.0.0.1:7890`, and `tilr` attaches to this tilrd without starting another
- **AND** SIGTERM makes tilrd exit 0 and the attached `tilr` print `[tilrd stopped]`

### Requirement: TCP port defaults to loopback 7890
The server SHALL listen for WebSocket clients on `--listen ADDR`, else `$TILR_LISTEN`, else `127.0.0.1:7890`, both when run by hand and when started by `tilr`. `--no-listen` SHALL turn the TCP port off and log `no TCP port: no tilr Cloud tunnel`. An address such as `0.0.0.0:7890` or `[::]:7890` SHALL be served as given (dual stack where the system allows it); what such clients must present is in the `access-control` spec.

**Verified by**: `local::the_socket_is_the_users_alone_and_asks_no_token`, `local::no_listen_serves_the_socket_alone`, `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it` and itests `UC-030` (`uc_030.py::a_tilrd_tilr_starts_listens_on_loopback_only`, `uc_030.py::a_bare_tilrd_listens_on_loopback_only`), `UC-032` (`uc_032.py::ipv6_loopback_serves_ipv6_only`, `uc_032.py::dual_stack_serves_both_loopbacks`).

#### Scenario: Loopback only
- **WHEN** a tilrd runs with the default `--listen` and another host connects to its port 7890
- **THEN** the connection is refused, while `127.0.0.1:7890` on the machine accepts it

#### Scenario: Socket only
- **WHEN** tilrd runs with `--no-listen`
- **THEN** its log has no `listening on ws://` line and says `no TCP port`, and `tilrd ps` over the socket works

### Requirement: A taken port
When the TCP address cannot be bound, a server started without `--lock-fd` (by hand or by a service manager) SHALL stop before taking the Unix socket, with exit status 1 and an error naming the address, leaving no socket. A server started by `tilr` (with `--lock-fd`) SHALL instead log a warning and go on with the Unix socket alone, with no TCP port and no Cloud tunnel.

**Verified by**: `local::a_tcp_port_that_is_taken_stops_the_server_with_a_clear_error`, `client::a_server_tilr_starts_goes_on_without_tcp_when_the_port_is_taken` and itests `UC-033` (`uc_033.py::a_taken_port_is_a_clear_error`, `uc_033.py::a_bare_tilrd_next_to_a_running_one_says_the_default_port_is_taken`, `uc_033.py::a_tilrd_tilr_starts_goes_on_without_the_taken_port`).

#### Scenario: Service on a taken port
- **WHEN** another program holds 127.0.0.1:P and the user runs `tilrd --listen 127.0.0.1:P`
- **THEN** tilrd exits 1 within seconds with `127.0.0.1:P is taken: another tilrd, or another program, listens there; pick another port with --listen, or none with --no-listen`, and no socket is left

#### Scenario: Other bind error
- **WHEN** the address cannot be bound for another reason
- **THEN** the error is `cannot listen on <addr>: <error>; pick another port with --listen, or none with --no-listen`

#### Scenario: Server tilr starts on a taken port
- **WHEN** 127.0.0.1:7890 is held by another program and the user runs `tilr new -d`
- **THEN** the command succeeds, `tilrd.log` has `127.0.0.1:7890 is taken: …; serving the Unix socket only (no Tilr app, remote clients or tilr Cloud tunnel through this server)` and no `listening on ws://` line

### Requirement: Unix socket location
The Unix socket SHALL be `--socket PATH`, else `$TILR_SOCKET`, else `tilrd.sock` in the runtime directory: `$TILR_RUNTIME_DIR`, else `$XDG_RUNTIME_DIR/tilr`, else `tilr-<euid>` in the temporary directory (`/tmp` on Linux, `$TMPDIR` on macOS); empty variables SHALL count as unset. Next to the socket SHALL be its lock file `<socket>.lock` and, for a server `tilr` started, its log `<socket stem>.log` (`tilrd.log`).

**Verified by**: `local::the_socket_is_the_users_alone_and_asks_no_token` and itests `UC-005` (`uc_005.py::on_a_clean_system_tilr_makes_its_directories_private`), `UC-010` (`uc_010.py::tilr_without_a_running_daemon_starts_tilrd_detached_and_connects`).

#### Scenario: No XDG_RUNTIME_DIR
- **WHEN** neither `TILR_RUNTIME_DIR` nor `XDG_RUNTIME_DIR` is set on Linux and the user runs `tilr new -d`
- **THEN** `/tmp/tilr-<uid>/` holds `tilrd.sock`, `tilrd.sock.lock` and `tilrd.log`

### Requirement: Runtime directory and socket modes
The server SHALL make the socket's directory 0700 when it creates it; for the default socket path it SHALL also set an existing runtime directory to 0700 and SHALL refuse it with `<dir> belongs to another user` when another uid owns it (`tilr` does the same before starting a server). The socket SHALL be 0600, the lock file 0600 with the holding server's pid in it, and `tilrd.log` 0600. The per-connection peer-uid check is in the `access-control` spec.

**Verified by**: `local::the_socket_is_the_users_alone_and_asks_no_token` and itests `UC-005` (`uc_005.py::on_a_clean_system_tilr_makes_its_directories_private`), `UC-034` (`uc_034.py::another_user_cannot_use_the_socket`).

#### Scenario: Modes after a start
- **WHEN** `tilr new -d` started a server on the default socket
- **THEN** the runtime directory is 0700, `tilrd.sock` and `tilrd.sock.lock` are 0600, and the lock file holds the server's pid

#### Scenario: Another user
- **WHEN** uid 65534 runs `tilr --socket <socket> ls` against the user's socket
- **THEN** it fails with `Permission denied`

### Requirement: Socket required or optional
The Unix socket SHALL be served unless `--no-socket` (which conflicts with `--socket`); `--no-socket` with `--no-listen` SHALL fail with `nothing to listen on: --no-socket and --no-listen`. A socket named with `--socket`, or the only way in (`--no-listen`), SHALL be required: failing to take it stops the server with `the Unix socket <path>: <why>`. Otherwise failing to take it SHALL only log `no Unix socket for local clients: <why>` and the server SHALL serve TCP alone.

**Verified by**: `local::no_listen_serves_the_socket_alone` and `local::with_a_tcp_port_a_socket_in_use_is_only_a_warning`, and itests `UC-037` (`uc_037.py::a_socket_path_longer_than_sun_path_is_a_clear_error`).

#### Scenario: App's server next to tilr's
- **WHEN** a tilrd serves the default socket and a second tilrd starts with `--listen 127.0.0.1:0` and no `--socket`
- **THEN** the second runs on TCP and logs `no Unix socket for local clients: another tilrd …`, and the first still serves the socket

#### Scenario: Named socket in use
- **WHEN** a second tilrd starts with `--listen 127.0.0.1:0 --socket <the first one's socket>`
- **THEN** it exits non-zero

### Requirement: Socket path length
A socket path longer than a Unix socket address holds (107 bytes on Linux, 103 on macOS) SHALL be refused before anything is created or started, by both `tilr` and `tilrd`, with `the socket path <path> is N bytes, longer than a Unix socket's M: set TILR_SOCKET (or TILR_RUNTIME_DIR, or XDG_RUNTIME_DIR) to a shorter one`.

**Verified by**: `local::a_socket_path_too_long_is_a_clear_error`, `client::a_socket_path_too_long_is_a_clear_error` and itests `UC-037` (`uc_037.py::a_socket_path_longer_than_sun_path_is_a_clear_error`).

#### Scenario: Deep runtime directory
- **WHEN** `XDG_RUNTIME_DIR` makes the socket path 130 bytes long and the user runs `tilr new -d` or `tilrd --no-listen`
- **THEN** each exits non-zero naming the length and `TILR_SOCKET`, and no tilrd runs
- **AND** a bare `tilrd --listen 127.0.0.1:0` serves TCP and logs `no Unix socket for local clients` with that message

### Requirement: One server per socket
A server SHALL hold an exclusive `flock` on `<socket>.lock` for as long as it runs, a reload included (see the `reload` spec), and SHALL write its pid into it. A server that cannot take the lock SHALL not touch the socket and SHALL fail (or, where the socket is optional, warn) with `another tilrd (pid N) serves it`. A lock file replaced between opening and locking SHALL be retried up to 5 times.

**Verified by**: `local::of_servers_started_at_once_one_serves_the_socket` and `local::a_reload_keeps_the_socket_and_its_lock`.

#### Scenario: Eight servers at once
- **WHEN** eight `tilrd --no-listen` start at once on one socket
- **THEN** exactly one keeps running, the lock file holds its pid, and the others exit non-zero saying `another tilrd`

### Requirement: What is found at the socket path
Holding the lock, a server SHALL remove a socket at its path that belongs to the user and refuses connections (left by a server killed with SIGKILL), logging `removing the socket of a server that is gone`, and listen there. It SHALL remove nothing and fail when the path holds a file that is not a socket, a socket of another uid, or a socket some program accepts on. `tilr` SHALL refuse a non-socket file at the path the same way before starting anything.

**Verified by**: `local::a_socket_left_by_a_killed_server_is_replaced`, `local::a_file_at_the_path_is_left_alone_with_a_clear_error`, `local::another_programs_listener_at_the_path_is_left_alone_with_a_clear_error`, `client::a_socket_left_by_a_killed_server_is_replaced_by_the_next_tilr`, `client::a_file_in_the_way_is_left_alone_with_a_clear_error` and itests `UC-014` (`uc_014.py::after_a_sigkill_the_next_tilr_replaces_the_socket_and_starts_a_new_daemon`), `UC-015` (`uc_015.py::another_programs_listener_at_the_socket_is_left_alone`, `uc_015.py::a_file_that_is_not_a_socket_is_left_alone`).

#### Scenario: Socket of a killed server
- **WHEN** a tilrd was killed with SIGKILL, leaving its socket, and the user runs `tilr new -d`
- **THEN** a new tilrd with another pid serves the socket and its log says `removing the socket of a server that is gone`

#### Scenario: Another program's listener
- **WHEN** another program listens on the socket path and tilrd or `tilr` starts
- **THEN** tilrd fails with `something that is not a tilrd listens on <path> (nothing was removed): stop it, or set TILR_SOCKET to another path` (and `tilr` as the `tilr-cli` spec says), and the other program's socket still accepts connections

#### Scenario: A file that is not a socket
- **WHEN** a regular file is at the socket path
- **THEN** tilrd and `tilr` fail with `<path> is in the way: it is not a socket (nothing was removed)` and the file is unchanged

#### Scenario: Another user's socket
- **WHEN** a socket owned by another uid is at the path
- **THEN** tilrd fails with `<path> belongs to uid N, not to this user (nothing was removed)`

### Requirement: tilr starts tilrd detached
When a command that starts the server (see the `tilr-cli` spec) finds nothing accepting on the local socket, `tilr` SHALL make the socket's directory (0700), take the socket's lock itself and start `tilrd --socket <socket> --lock-fd <fd>` with the locked descriptor inherited, which the server adopts as its lock. The server SHALL run detached: a grandchild of `tilr`, leading a session of its own with no controlling terminal. `tilr` SHALL wait up to 15 s for the socket to accept connections.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it` and itests `UC-010` (`uc_010.py::tilr_without_a_running_daemon_starts_tilrd_detached_and_connects`).

#### Scenario: First tilr on an empty machine
- **WHEN** no tilrd runs and the user runs `tilr` in a terminal
- **THEN** exactly one tilrd runs, its session id is its pid, it has no controlling terminal and is not a child of `tilr`
- **AND** after the client's terminal hangs up, that tilrd is still running

#### Scenario: The started process
- **WHEN** `tilr` starts tilrd
- **THEN** tilrd's stdin is `/dev/null`, its stdout and stderr are appended to `tilrd.log` (0600) next to the socket, its working directory is `/`, and `TILR_PANE`, `TILR_SOCK`, `TILR_SERVER` and `NOTIFY_SOCKET` are removed from its environment

#### Scenario: Which tilrd
- **WHEN** `tilr` looks for the executable
- **THEN** it takes `tilrd` next to the `tilr` executable, else the first executable `tilrd` on `PATH`, else `$TILRD`, and with none fails with `no tilrd to start: none next to <tilr>, none on PATH, and $TILRD is not set`

#### Scenario: Server fails to start
- **WHEN** the started tilrd exits before it listens, or nothing listens after 15 s
- **THEN** `tilr` fails with `tilrd stopped before it listened on <socket>:` or `no tilrd listens on <socket> after 15s; its log (<log>):`, followed by the last lines of the log

### Requirement: Concurrent starts start one server
Of several `tilr` started at once with no server running, only the one that takes the lock SHALL start a tilrd; the others SHALL wait for the socket and use that server, and no second tilrd SHALL even be started.

**Verified by**: `client::tilr_started_many_times_at_once_starts_one_server` and itests `UC-012` (`uc_012.py::tilr_started_eight_times_at_once_starts_one_daemon`, `uc_012.py::two_terminals_running_tilr_at_once_share_one_daemon`).

#### Scenario: Six at once
- **WHEN** six `tilr new -d` start at once on an empty machine
- **THEN** all succeed, `tilr ls -q` lists all six panes, and the log has one `Unix socket for local clients` line and no `another tilrd`

### Requirement: Ready notification for a service manager
When `$NOTIFY_SOCKET` is set, the server SHALL send the datagram `READY=1` to it (a path, or `@name` in the abstract namespace on Linux) once it listens and has resumed any reloaded state; a failure to send SHALL only be logged.

**Verified by**: `local::a_service_manager_is_told_when_the_server_is_ready` and itests `UC-020` (`uc_020.py::service_install_writes_a_notify_user_unit_and_enables_it`).

#### Scenario: systemd Type=notify
- **WHEN** tilrd starts with `NOTIFY_SOCKET` naming a datagram socket
- **THEN** that socket receives exactly `READY=1`

### Requirement: SIGHUP is ignored and panes have their own sessions
The server SHALL catch SIGHUP and keep running, logging `SIGHUP ignored: the server keeps running without its terminal` on the first one only; reloading is SIGUSR1's, not SIGHUP's. Each pane process SHALL run in a session of its own (`setsid`), so a hang-up or Ctrl-C sent to the terminal the server was started from does not reach the panes or their shells.

**Verified by**: `reconnect::a_server_started_from_a_terminal_outlives_the_terminal_hanging_up` and itests `UC-013` (`uc_013.py::tilr_in_an_ssh_session_leaves_tilrd_and_its_shell_when_the_session_drops`, `uc_013.py::tilrd_started_from_an_ssh_login_shell_survives_the_session_dropping`, `uc_013.py::tilrd_started_with_setsid_keeps_its_panes_when_the_ssh_session_drops`).

#### Scenario: tilrd & in an SSH login
- **WHEN** `tilrd &` was started from a login shell on a pty and the pty hangs up
- **THEN** the server, its pane process and the shell are still running, and a client reconnecting sees the shell's variables as they were
- **AND** SIGTERM to the server afterwards still ends the shell and the server

### Requirement: Stopping
SIGTERM and SIGINT SHALL stop the server: it SHALL stop accepting work, log `shutting down, closing N pane(s)`, remove the agent sockets, end running execs (reason `shutdown`, see the `exec` spec), close its pipes to the pane processes so each pane process exits and its pty closes (hanging up its shell), remove the host socket, remove the Unix socket (only if it is still a socket) and the lock file (only if it is still the file it locked), then let go of the lock and exit 0.

**Verified by**: `local::a_server_that_stops_removes_its_socket_and_lock`, `client::kill_server_ends_the_shells_and_removes_the_socket` and itests `UC-017` (`uc_017.py::kill_server_leaves_no_process_and_no_socket`).

#### Scenario: kill-server with three panes
- **WHEN** a client is attached, three panes run (one `sleep 1000`) and the user runs `tilr kill-server`
- **THEN** no tilrd, pane process or program in a pane is left, `tilrd.sock` and `tilrd.sock.lock` are gone, and `tilr ls` says `no tilrd runs`

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

### Requirement: State directory
What outlives a restart SHALL be kept in the state directory: `$TILR_STATE_DIR`, else `$XDG_STATE_HOME/tilr`, else `~/.local/state/tilr`, created 0700 (its parents as usual) when missing. The server's id SHALL be the UUID in `server-id` there (0600), made on first start and written aside and hard-linked into place so concurrent starts agree; a server that cannot keep it SHALL warn and use an id that lasts until it stops. The token file kept there is the `access-control` spec's.

**Verified by**: itests `UC-005` (`uc_005.py::on_a_clean_system_tilr_makes_its_directories_private`).

#### Scenario: Clean home
- **WHEN** a user with no `~/.local/state` runs `tilr new -d`
- **THEN** `~/.local/state/tilr` is 0700 and the files in it are 0600

### Requirement: Lock and log files are not opened through links
The server and `tilr` SHALL open the socket's lock file (`<socket>.lock`) and `tilr` the server's log (`tilrd.log`) without following a symbolic link (`O_NOFOLLOW`), and SHALL refuse one that is not a regular file of the user's; the lock file MUST NOT be truncated before the lock is held.

**Verified by**: `client::a_symlinked_lock_file_is_refused` (crates/tilr/tests/client.rs, the lock and the log), `local::a_symlinked_lock_file_is_refused` (crates/tilrd/tests/local.rs).

#### Scenario: Symlinked lock file
- **WHEN** `tilrd.sock.lock` is a symbolic link to another file
- **THEN** `tilr new -d` and `tilrd` fail with an error saying a symbolic link is not followed, no server serves the socket and the link target is unchanged

#### Scenario: Symlinked log
- **WHEN** `tilrd.log` is a symbolic link to another file and `tilr` starts a server
- **THEN** `tilr` fails with that error and the link target is unchanged

### Requirement: An existing state directory is made private
At start the server SHALL set an existing state directory owned by its euid to 0700 and SHALL NOT use one owned by another user (`<dir> belongs to another user`), before it reads or writes `server-id`, the token or the Cloud link there.

**Verified by**: `auth::an_existing_state_directory_is_made_private`.

#### Scenario: Loose state directory
- **WHEN** the state directory exists with mode 0755 and holds `server-id` and the token, and `tilrd` starts
- **THEN** the directory is 0700 and the server keeps its id and token

### Requirement: Signals while the server starts
From the moment the server binds its TCP port or claims its Unix socket until its signal handlers are in place, SIGTERM, SIGINT, SIGHUP and SIGUSR1 MUST NOT end the server by their default action: each SHALL be recorded and acted on once the handlers are in place, SIGTERM and SIGINT as a stop (see Stopping: the Unix socket and its lock file removed, exit 0), SIGHUP ignored, SIGUSR1 one reload.

**Verified by**: `local::a_sigterm_while_the_server_starts_stops_it_cleanly`, `local::a_sigint_while_the_server_starts_stops_it_cleanly`, `local::a_sighup_while_the_server_starts_is_ignored`.

#### Scenario: SIGTERM between the bind and the handlers
- **WHEN** `tilrd` starts with the failpoint `supervisor-before-signals=sleep:1000`, its socket accepts a connection and it receives SIGTERM during the sleep
- **THEN** it exits 0, the log has `shutting down`, and neither `tilrd.sock` nor `tilrd.sock.lock` is left

#### Scenario: Ctrl-C between the bind and the handlers
- **WHEN** the same is done with SIGINT
- **THEN** it exits 0 and neither the socket nor the lock file is left

#### Scenario: SIGHUP between the bind and the handlers
- **WHEN** it receives SIGHUP during the sleep
- **THEN** it keeps running and serves a client after the sleep, and the log has `SIGHUP ignored`

## Notes

- Partly untested: the "logged once" part of the SIGHUP requirement; in "tilr starts tilrd detached" the start timeout, the log-tail messages and the `$TILRD` fallback; the other-bind-error and another-user's-socket messages.
- The "Verified by" line is written as an OpenSpec metadata line (`**Verified by**: …`) so that it does not count toward the 500-character requirement-text limit of `openspec validate --strict`.
- The TCP port is bound before the Unix socket is claimed (crates/tilrd/src/supervisor.rs `run`), which is why a taken port leaves no socket.
- For a non-default socket path the server makes the directory 0700 only when it creates it; an existing directory keeps its mode (crates/tilrd/src/local.rs `claim`).
