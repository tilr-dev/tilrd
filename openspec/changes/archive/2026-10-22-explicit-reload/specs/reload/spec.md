## ADDED Requirements

### Requirement: Replacing the executable does not request reload
Replacing the installed executable SHALL NOT trigger a reload. Only an explicit request SHALL initiate the successor checks and reload. The obsolete `--no-auto-reload` option SHALL be rejected as an unknown argument.

**Verified by**: `reload::replacement_waits_for_an_explicit_request_and_keeps_the_shells`, `reload::the_obsolete_auto_reload_option_is_rejected`.

#### Scenario: Replace without a request
- **WHEN** the installed executable is atomically replaced while a client is connected
- **THEN** the same connection keeps serving and the server has not initiated a reload

#### Scenario: Explicit request after replacement
- **WHEN** the executable is replaced and the server receives `SIGUSR1`
- **THEN** the server checks its successor and reloads with the same shells

#### Scenario: Removed option
- **WHEN** `tilrd --no-auto-reload` runs
- **THEN** argument parsing rejects the unknown option

### Requirement: Installation requests reload explicitly
`tilrd reload` SHALL send `SIGUSR1` only to this user's supervisors running from that installation path, excluding pane and command processes and other installations. It SHALL exit successfully without starting a server when none is running. Update and installation SHALL use this explicit request after the atomic rename.

**Verified by**: `reload::an_explicit_request_selects_only_its_installation_without_a_socket`, `reload::an_explicit_request_with_no_server_starts_nothing`; itests `UC-003` (`uc_003.py::installing_a_new_release_reloads_the_running_server_and_keeps_its_shells`).

#### Scenario: Select the installed server
- **WHEN** servers from two installation paths are running and `tilrd reload` runs from one installation
- **THEN** only that installation's supervisor receives a reload request and keeps its panes

#### Scenario: No running server
- **WHEN** `tilrd reload` runs with no supervisor from its installation
- **THEN** it exits 0 and starts no server

### Requirement: Reload checks and executes the installation path
The supervisor SHALL check the executable at its installation path and exec that same path. Replacement between check and exec is an accepted risk, without locks, pinned descriptors or private copies.

#### Scenario: Replacement between check and exec
- **WHEN** another atomic replacement occurs after the successor checks and before exec
- **THEN** the server executes the file now at the installation path, which may differ from the checked file
- **AND** a successor that fails after exec follows the existing reload failure behavior

## MODIFIED Requirements

### Requirement: SIGUSR1 reloads the server
Only on an explicit reload request (`SIGUSR1`), including one sent after `tilr update` or installation, the supervisor SHALL exec the executable it was started from in place and then have every pane process do the same; every pane SHALL keep its shell (same pid) with its variables and jobs, its title, directory and creation time.


**Verified by**: `reload::panes_survive_a_reload_asked_for_with_a_signal`, `reload::every_pane_keeps_its_own_shell_across_a_reload`, `reload::the_title_and_directory_of_a_pane_are_kept_across_a_reload`.

#### Scenario: Reload on a signal
- **WHEN** a pane's shell has set `KEPT=yes` and the server receives `SIGUSR1`
- **THEN** after the client reconnects the pane lists with the same shell pid and `test "$KEPT" = yes` exits 0

### Requirement: The successor must run and read the state formats
Before handing over, the supervisor SHALL run the executable on disk as `tilrd state-formats` (10 s limit) and SHALL go on with the reload only if it exits 0, prints `supervisor-state` and `pane-state` versions equal to the ones this server writes (2 and 3 at HEAD; 0 counts as 1) and prints `reload-check 1`. Else it SHALL log `not reloading: ...` and carry on with its code and every connection; so no release before the reload check is reloaded into.

**Verified by**: `reload::a_broken_executable_is_not_reloaded_into`, `reload::an_executable_that_reads_another_state_format_is_not_reloaded_into`, unit test `reload::tests::check_formats_rejects_every_format_mismatch` (crates/tilrd/src/reload.rs); itests `UC-067` (`uc_067.py::a_version_with_another_state_format_is_not_reloaded_into_and_the_shells_live_on`).

#### Scenario: Executable that does not run
- **WHEN** the executable is replaced by a script that exits 1 and an explicit reload is requested
- **THEN** the server logs `not reloading`, stays up, and the same client runs a command in its pane

#### Scenario: Another state format
- **WHEN** the new executable prints `supervisor-state 2` and `pane-state 2`
- **THEN** the server logs `reads pane-state format 2, this server writes 3` and does not reload; a later executable printing the matching formats is reloaded into

#### Scenario: An executable without the reload check
- **WHEN** the new executable prints this server's formats but no `reload-check 1`
- **THEN** the server logs `not reloading: ... cannot show that it can resume` and carries on

### Requirement: A successor that cannot take the socket
If the successor does not print `unix-socket 1`, a server that serves its unix socket SHALL NOT reload into it (`not reloading: <exe> cannot take over the Unix socket this server serves`), whether or not it also has a TCP port; the socket and its lock stay served. A server without a unix socket (`--no-socket`) reloads as before.

**Verified by**: `local::a_server_on_its_socket_alone_does_not_reload_into_one_that_cannot_take_it`, `local::a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket`.

#### Scenario: Socket alone, older successor
- **WHEN** a server started with `--no-listen` has its executable replaced by one whose `state-formats` lacks `unix-socket 1` and an explicit reload is requested
- **THEN** the log says `cannot take over the Unix socket this server serves`, the server keeps running and `tilrd ps` works

#### Scenario: Socket and port, older successor
- **WHEN** a server with a TCP port and its unix socket has its executable replaced by one whose `state-formats` lacks `unix-socket 1` and an explicit reload is requested
- **THEN** it does not reload, and `tilrd ps` works through the socket

### Requirement: SOCKS sessions and agent streams end at a reload
A SOCKS session or an SSH agent stream open at a reload SHALL end for both sides (the service and the program read EOF), new ones SHALL work afterwards, and the panes' agent socket SHALL be made again at the same path.

**Verified by**: `agent::the_panes_socket_works_after_a_reload`; itests `UC-064` (`uc_064.py::a_socks_session_open_during_an_update_ends_for_the_service_and_a_new_one_works`, `uc_064.py::an_agent_stream_open_during_an_update_ends_for_the_program_and_the_next_one_works`).

#### Scenario: SOCKS session through an update
- **WHEN** a SOCKS session to an echo service is open and the executable is replaced and an explicit reload is requested
- **THEN** the service sees its connection end, the client's side ends too, and a new session echoes 1 MB

### Requirement: A file write cut by a reload leaves the file as it was
An `FsWrite` cut by a reload SHALL fail, leave the target file as it was and leave no temporary file; the same write retried SHALL succeed (see the `files` spec).

**Verified by**: itests `UC-064` (`uc_064.py::a_file_write_broken_by_an_update_leaves_the_file_as_it_was_and_the_retry_works`).

#### Scenario: File write cut by an update
- **WHEN** an 8 MiB chunked `FsWrite` is in progress and the executable is replaced and an explicit reload is requested
- **THEN** the target keeps its old content, no `.tilr-write-*` file is left, and the retried write succeeds

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

### Requirement: A refused reload keeps the socket serving
When a reload is refused or fails before the handover, the Unix socket file and its lock SHALL stay in place and keep serving, so that `tilr` MUST NOT start a second server.

**Verified by**: `local::a_refused_reload_keeps_the_socket_and_its_lock`; itests `UC-06B` (`uc_06b.py::a_refused_reload_keeps_the_socket_and_tilr_finds_the_same_server`).

#### Scenario: Successor fails its check
- **WHEN** a reload is refused because the successor fails the reload check
- **THEN** the socket file and its lock are in place, a local client connects through the socket, and no second `tilrd` runs

#### Scenario: A server started by tilr
- **WHEN** a server `tilr` started (with `--socket` and `--lock-fd`) has its executable replaced by a release that crashes resuming and an explicit reload is requested
- **THEN** the reload is refused, `tilr ls` lists the same panes, and a new `tilr` attaches to the same server with the same shell

## REMOVED Requirements

### Requirement: A replaced executable is reloaded into automatically
**Reason**: Reload is requested explicitly after installation or update.
**Migration**: Send `SIGUSR1` after replacing the executable.

### Requirement: --no-auto-reload turns the watch off
**Reason**: There is no executable watch.
**Migration**: Remove the option from startup arguments.

### Requirement: A second replacement during a reload costs no shell
**Reason**: The automatic reload workflow is removed.
**Migration**: Install a release atomically before requesting reload.
