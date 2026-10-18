# reload Specification

## Purpose

Covers how a running `tilrd` picks up a new executable without ending any shell: what triggers a reload, the in-place exec of the supervisor and then of every pane process, the state files handed over in the runtime directory, the checks made before handing over, what carries over (sockets, panes, screens) and what does not (client connections, execs, open streams, `cloudflared`), and what happens when a reload fails halfway. Fetching and installing a release is the `update` spec; Exec, files, SOCKS and the agent have their own specs, only their fate at a reload is here.

## Requirements

### Requirement: SIGUSR1 reloads the server
On `SIGUSR1` the supervisor SHALL exec the executable it was started from in place and then have every pane process do the same; every pane SHALL keep its shell (same pid) with its variables and jobs, its title, directory and creation time.

**Verified by**: `reload::panes_survive_a_reload_asked_for_with_a_signal`, `reload::every_pane_keeps_its_own_shell_across_a_reload`, `reload::the_title_and_directory_of_a_pane_are_kept_across_a_reload`.

#### Scenario: Reload on a signal
- **WHEN** a pane's shell has set `KEPT=yes` and the server receives `SIGUSR1`
- **THEN** after the client reconnects the pane lists with the same shell pid and `test "$KEPT" = yes` exits 0

### Requirement: Reloads work with no panes and again and again
A reload with no pane open SHALL work and the server SHALL open panes afterwards; a server SHALL be able to reload any number of times in a row.

**Verified by**: `reload::a_reload_with_no_panes_open_works`, `reload::a_server_can_reload_again_and_again`.

#### Scenario: No panes
- **WHEN** a server with no pane gets `SIGUSR1`
- **THEN** it is running, lists no pane, and a new pane runs `true` with exit code 0

### Requirement: Screens, scrollback and programs carry over
A pane's screen, terminal modes (alternate screen, mouse, hidden cursor), scrollback and scroll position SHALL be the same after a reload, and its programs SHALL keep answering keys, signals and job control.

**Verified by**: `scroll::history_survives_a_reload`, `scroll::a_scrolled_view_stays_scrolled_through_a_reload`; itests `UC-061` (`uc_061.py::vim_keeps_its_screen_modes_and_buffer_across_a_reload_and_writes_it`, `uc_061.py::htop_keeps_its_modes_across_a_reload_and_answers_keys_and_the_mouse`), `UC-062` (`uc_062.py::ctrl_z_and_fg_after_a_reload_stop_and_resume_a_job`, `uc_062.py::ctrl_c_after_a_reload_interrupts_a_foreground_sleep`, `uc_062.py::a_sleep_running_through_a_reload_is_interrupted_by_ctrl_c`).

#### Scenario: vim across a reload
- **WHEN** vim with an unsaved buffer runs in a pane and the server reloads
- **THEN** the alternate screen, mouse tracking and hidden cursor are as before and `:wq` writes the buffer

#### Scenario: Ctrl+C after a reload
- **WHEN** a `sleep` runs in the foreground through a reload and Ctrl+C is typed
- **THEN** the sleep is interrupted and the shell prompts again

### Requirement: A replaced executable is reloaded into automatically
The supervisor SHALL check the file it was started from once a second and SHALL reload, logging `executable changed on disk`, once its inode, size or mtime has changed and looked the same on two checks in a row. The path is the one resolved at start, so repointing a symlink SHALL NOT be noticed.

**Verified by**: `reload::panes_survive_the_executable_being_replaced_on_disk`; itests `UC-003` (`uc_003.py::installing_a_new_release_reloads_the_running_server_and_keeps_its_shells`).

#### Scenario: New file renamed into place
- **WHEN** a new executable is written next to the running one and renamed over it
- **THEN** within a few seconds the server logs `executable changed on disk`, reloads, and the panes survive

### Requirement: --no-auto-reload turns the watch off
With `--no-auto-reload` the supervisor SHALL NOT reload when its executable is replaced; `SIGUSR1` SHALL still reload it.

**Verified by**: `reload::a_replaced_executable_is_ignored_when_auto_reload_is_off`.

#### Scenario: Auto reload off
- **WHEN** the server runs with `--no-auto-reload` and its executable is replaced
- **THEN** after 4 seconds the same connection still works and the log has no `reloading`

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

### Requirement: A reload keeps pids and command lines and leaves no state file
The supervisor and each pane process SHALL be exec'd with the arguments they were started with, keeping their pid and their `ps` command line, and the server SHALL log `resumed after reload with N pane(s)`. Each state file SHALL be removed as its successor reads it, so no `*.state` file is left.

**Verified by**: `reload::a_reload_shows_in_no_command_line_and_leaves_no_file_behind`; itests `UC-060` (`uc_060.py::update_from_the_cdn_keeps_every_program_in_the_panes_alive`).

#### Scenario: Same processes
- **WHEN** a server with one pane reloads
- **THEN** the pane's pid and the command lines of the supervisor and the pane process (`pane --id <id> ...`) are unchanged and no `.state` file is left in the runtime directory or `panes/`

### Requirement: State files are private and written whole or not at all
The supervisor SHALL leave its state in `<runtime dir>/server-<pid>.state` and a pane process in `<runtime dir>/panes/<pane id>.state`, mode 0600 in a private directory, each written to `.<name>.tmp` next to it, synced and renamed. A failed write (a full disk) SHALL remove the temporary file, and that process SHALL not reload and carry on as it was.

**Verified by**: itests `UC-065` (`uc_065.py::a_reload_with_no_room_for_its_state_keeps_the_server_and_its_panes`).

#### Scenario: No room for the supervisor's state
- **WHEN** the runtime directory's disk is full and the server receives `SIGUSR1`
- **THEN** the log has no `resumed after reload with`, the client stays connected, every pane and shell is as it was, and no state or temporary file is left

### Requirement: A pane that cannot save its state stays as it is
When the supervisor's state fits but a pane's does not, that pane process SHALL carry on in the code it has, and every pane SHALL live on with its shell; no half-written state file SHALL be left.

**Verified by**: itests `UC-065` (`uc_065.py::a_reload_with_room_for_the_servers_state_but_not_the_panes_keeps_every_pane`).

#### Scenario: One page free
- **WHEN** only 4 KiB is free on the runtime disk and the server reloads
- **THEN** every pane and its programs are as before and `state_files` is empty; with room again the next reload goes through

### Requirement: Stale state files are swept at start
A supervisor that starts or has just reloaded SHALL remove `server-*.state` in the runtime directory and `*.state` in `panes/`, and their `.*.state.tmp` temporaries, that are older than one hour.

#### Scenario: Leftover from a crashed reload
- **WHEN** a `server-1234.state` file two hours old is in the runtime directory and the server starts
- **THEN** the file is removed

### Requirement: The TCP listener carries over
A reload SHALL keep the TCP listening socket open across the exec, so the port is never closed and clients reconnect to the same address.

**Verified by**: `reload::panes_survive_a_reload_asked_for_with_a_signal`; itests `UC-003` (`uc_003.py::installing_a_new_release_reloads_the_running_server_and_keeps_its_shells`).

#### Scenario: Client on TCP 7890
- **WHEN** a client connected over `ws://127.0.0.1:7890` sees the server reload
- **THEN** it reconnects to the same port and finds its pane

### Requirement: The unix socket and its lock carry over
If the successor prints `unix-socket 1`, the unix socket's listener and its flock lock (`tilrd.sock.lock`) SHALL be handed over, so the socket stays at its path and locked throughout.

**Verified by**: `local::a_reload_keeps_the_socket_and_its_lock`.

#### Scenario: Reload with local clients
- **WHEN** a server listening on its unix socket reloads
- **THEN** `tilrd.sock` stays at its path, its lock stays held, and local clients reconnect to it

### Requirement: A successor that cannot take the socket
If the successor does not print `unix-socket 1`, a server that serves its unix socket SHALL NOT reload into it (`not reloading: <exe> cannot take over the Unix socket this server serves`), whether or not it also has a TCP port; the socket and its lock stay served. A server without a unix socket (`--no-socket`) reloads as before.

**Verified by**: `local::a_server_on_its_socket_alone_does_not_reload_into_one_that_cannot_take_it`, `local::a_server_with_a_port_does_not_reload_into_one_that_cannot_take_its_socket`.

#### Scenario: Socket alone, older successor
- **WHEN** a server started with `--no-listen` has its executable replaced by one whose `state-formats` lacks `unix-socket 1`
- **THEN** the log says `cannot take over the Unix socket this server serves`, the server keeps running and `tilrd ps` works

#### Scenario: Socket and port, older successor
- **WHEN** a server with a TCP port and its unix socket has its executable replaced by one whose `state-formats` lacks `unix-socket 1`
- **THEN** it does not reload, and `tilrd ps` works through the socket

### Requirement: Client connections end and clients reconnect
A reload SHALL end every client connection (sockets are not carried over); a client that reconnects SHALL find its panes as they were. Input the server had not read from a client socket when it exec'd is lost with it, but nothing sent SHALL be doubled.

**Verified by**: itests `UC-063` (`uc_063.py::a_client_typing_through_an_update_reconnects_and_nothing_typed_is_lost_or_doubled`).

#### Scenario: Typing through an update
- **WHEN** a client types numbered lines into `cat >>typed` every 20 ms through an update, re-sending lines whose send failed
- **THEN** after it reconnects every line is in the file once, in order

### Requirement: Pane input and output are not lost across a reload
Output a pane prints during a reload SHALL reach the client after it reconnects, and input the server took before the reload SHALL reach the shell exactly once, with the input latency marks kept.

**Verified by**: `reload::output_printed_during_a_reload_is_not_lost`, `reload::input_typed_just_before_a_reload_reaches_the_shell`, `metrics::the_last_input_and_input_still_waiting_survive_a_reload`.

#### Scenario: Input just before
- **WHEN** a client types a command and the server reloads at once
- **THEN** the shell runs the command once

### Requirement: A stuck pane process is closed, not carried over
A pane process that does not take the input already queued for it within 5 seconds SHALL be killed (log `pane process is stuck, closing it`) and its pane closed; the others SHALL be carried over. A reload asked for while a pane is starting SHALL be retried every 200 ms.

**Verified by**: itests `UC-068` (`uc_068.py::panes_created_while_the_server_reloads_are_whole_or_absent_and_every_reload_happens`).

#### Scenario: Panes created during reloads
- **WHEN** panes are created while the server reloads repeatedly
- **THEN** each pane is whole or absent and every reload happens

### Requirement: A pane that cannot hand over carries on
Before its own exec a pane process SHALL check its successor the same way, with a copy of its state (`panes/<pane id>.check.state`). A pane process whose reload fails before its exec, its check included, SHALL log `reload failed` (for a failed check with `the new executable could not resume` and the reason), remove its state files and carry on in the code it has with its shell; a pane process that crashes in its live resume SHALL be lost (its shell ends) and the other panes SHALL carry on.

**Verified by**: `reload::a_pane_that_cannot_hand_over_carries_on_in_the_code_it_has`, `reload::a_pane_whose_successor_fails_the_check_carries_on`, `reload::a_pane_that_crashes_taking_over_is_lost_and_the_others_carry_on`, `reload::a_shell_that_exits_after_a_reload_still_reports_its_exit_code`.

#### Scenario: Pane resume panics in the check
- **WHEN** the failpoint `pane-resume=panic` is set and the server reloads
- **THEN** the supervisor runs the new code, the pane process logs `reload failed` and `the new executable could not resume`, and the pane is still listed with the same shell, its variables and its screen

#### Scenario: One pane crashes taking over
- **WHEN** two panes are open and the first pane's process panics in its live resume after a reload (`pane-resume-live:<pane id>=panic`)
- **THEN** the server logs `pane process died`, lists only the second pane with its same shell and variables, and the first pane's shell exits

### Requirement: A reload that fails before the handover changes nothing
If anything fails before the supervisor's exec (its state, its descriptors, the exec), it SHALL log `reload failed: ...`, remove its state file and carry on as before: same client connections, panes and shells, and new panes open.

**Verified by**: `reload::a_reload_that_fails_before_the_handover_leaves_everything_as_it_was`.

#### Scenario: Failure just before exec
- **WHEN** the failpoint `reload-before-exec=error` is set and the server gets `SIGUSR1`
- **THEN** it logs `reload failed`, the same connection runs `test "$KEPT" = yes` with the same shell pid, and a new pane opens

### Requirement: No shell outlives a server lost in a reload
A successor that passed the reload check and then fails or crashes taking over SHALL end the server, as SHALL a supervisor killed mid-reload; every shell SHALL then exit, so none is left where no client can reach it. `SIGTERM` before or after a reload SHALL end the shells.

**Verified by**: `reload::a_version_that_crashes_taking_over_ends_the_server_and_its_shells`, `reload::a_server_killed_in_the_middle_of_a_reload_leaves_no_shell_behind`, `reload::stopping_the_server_after_a_reload_ends_the_shells`.

#### Scenario: Successor panics on resume
- **WHEN** the failpoint `supervisor-resume-live=panic` is set and the server reloads
- **THEN** the server process ends and the pane's shell exits

### Requirement: A second replacement during a reload costs no shell
When the executable is replaced again while a reload is under way, the server SHALL stay up with all its panes, whatever it makes of the second file.

**Verified by**: `reload::an_executable_replaced_again_during_a_reload_costs_no_shell`.

#### Scenario: Replaced again mid-reload
- **WHEN** the executable is replaced again while a reload sleeps 1.5 s before its exec
- **THEN** the panes survive, two panes list afterwards, and the log has no `pane process died`

### Requirement: Inherited processes are waited without leaks
Pane processes inherited through a reload SHALL be waited for as children (by a pidfd on Linux 5.3+), so that after any number of reloads the supervisor holds as many descriptors as before the first one and no orphan or zombie is left.

**Verified by**: itests `UC-069` (`uc_069.py::fifty_reloads_under_load_leave_no_orphans_and_no_descriptor_leaks`), `UC-060` (`uc_060.py::update_from_the_cdn_keeps_every_program_in_the_panes_alive`).

#### Scenario: Fifty reloads under load
- **WHEN** the server reloads 50 times in a row with busy panes
- **THEN** the supervisor's descriptor count equals the count before the first reload and no orphan process is left

### Requirement: cloudflared is stopped before the exec
Before its exec the supervisor SHALL stop its `cloudflared` (killed if it does not exit in time) so it never runs alongside the successor's; the successor SHALL start its own, and a reload that fails SHALL give the old supervisor its `cloudflared` back.

**Verified by**: `cloud::a_reload_replaces_cloudflared_with_one_of_the_new_server`.

#### Scenario: Linked server reloads
- **WHEN** a server with its tunnel up gets `SIGUSR1`
- **THEN** the new supervisor connects to Cloud again with a different `cloudflared` pid and the old `cloudflared` is gone

### Requirement: Execs end at a reload
Execs SHALL NOT be handed over: each running exec and its process group SHALL be killed and its client SHALL get `ExecExit` with `reason` `reload` and `signal` 9 before the connection goes; see the `exec` spec.

**Verified by**: `exec::a_reload_ends_the_programs_and_their_clients_hear_why`.

#### Scenario: Exec at a reload
- **WHEN** a client runs `sh -c 'sleep 60 & ...; wait'` as an exec and the server gets `SIGUSR1`
- **THEN** the client gets `ExecExit` with `reason` `reload` and `signal` 9, the program and its child are gone, and an exec after reconnecting works

### Requirement: SOCKS sessions and agent streams end at a reload
A SOCKS session or an SSH agent stream open at a reload SHALL end for both sides (the service and the program read EOF), new ones SHALL work afterwards, and the panes' agent socket SHALL be made again at the same path.

**Verified by**: `agent::the_panes_socket_works_after_a_reload`; itests `UC-064` (`uc_064.py::a_socks_session_open_during_an_update_ends_for_the_service_and_a_new_one_works`, `uc_064.py::an_agent_stream_open_during_an_update_ends_for_the_program_and_the_next_one_works`).

#### Scenario: SOCKS session through an update
- **WHEN** a SOCKS session to an echo service is open and the executable is replaced
- **THEN** the service sees its connection end, the client's side ends too, and a new session echoes 1 MB

### Requirement: A file write cut by a reload leaves the file as it was
An `FsWrite` cut by a reload SHALL fail, leave the target file as it was and leave no temporary file; the same write retried SHALL succeed (see the `files` spec).

**Verified by**: itests `UC-064` (`uc_064.py::a_file_write_broken_by_an_update_leaves_the_file_as_it_was_and_the_retry_works`).

#### Scenario: File write cut by an update
- **WHEN** an 8 MiB chunked `FsWrite` is in progress and the executable is replaced
- **THEN** the target keeps its old content, no `.tilr-write-*` file is left, and the retried write succeeds

### Requirement: Temporary files of cut writes are swept at start
Each file write SHALL note its temporary file in `<runtime dir>/writes/` (`<pid>-<random>`). A server that starts or has just reloaded SHALL, before any write, remove the temporary files (absolute paths, names containing `.tilr-write-`) noted by a process that is gone or by itself, and the notes.

**Verified by**: itests `UC-064` (`uc_064.py::a_file_write_broken_by_an_update_leaves_the_file_as_it_was_and_the_retry_works`).

#### Scenario: Write cut by an exec
- **WHEN** a reload execs the supervisor while a write's temporary file exists and its note names the supervisor's pid
- **THEN** the successor removes the temporary file and the note at start

### Requirement: Rollback and roll-forward keep the panes
Reloading into an older executable and forward again SHALL keep the panes and their programs, as long as each executable reads the state the running one writes.

**Verified by**: itests `UC-066` (`uc_066.py::rolling_back_and_forward_again_keeps_the_panes_and_their_programs`).

#### Scenario: Back and forth
- **WHEN** the server updates to the next release, back to the current one and forward again
- **THEN** after each step it reports the expected version, keeps its pid and every program in its panes

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

## Notes

- Requirements without a test: Stale state files are swept at start.
- Write-whole state files and the write sweep have no cargo integration test; they are covered by itests UC-064 / UC-065 and by unit tests in `crates/tilrd/src/reload.rs` (`a_state_file_is_replaced_whole_and_nothing_else_is_left`, `a_state_file_that_cannot_be_written_leaves_nothing_behind`).
- Also covered: `reload::stopping_the_server_ends_the_shells` (SIGTERM without a reload); `uc_061.py::htop_keeps_its_modes_across_a_reload_and_answers_keys_and_the_mouse`, `uc_062.py::ctrl_c_after_a_reload_interrupts_a_foreground_sleep`, `uc_062.py::a_sleep_running_through_a_reload_is_interrupted_by_ctrl_c`.
- UC-064 and UC-065 were red until 2026-10-17 per itests/USE_CASES.md:136-137 and are green at HEAD; no known red reload case.
- Failpoints (`TILR_FAILPOINTS`, `TILR_FAILPOINTS_FILE`; actions `error`, `panic`, `exit`, `kill`, `sleep:MS`) used here: `reload-before-exec`, `supervisor-resume` and `pane-resume`, `pane-resume:<pane id>` (in the check run of a successor), `supervisor-resume-live` and `pane-resume-live`, `pane-resume-live:<pane id>` (in the live resume after the exec), `pane-reload-before-exec`. Only with the `failpoints` feature; a release has none.
- The check run is not checked from outside for binding or locking nothing ("The check process resumes without taking over"): the tests see that the server keeps its socket, connections and panes through it.
- The pidfd wait is Linux-only; elsewhere an inherited child is waited on a blocking thread.
- README.md "Reloading without closing shells" says the server reloads "about a second after the file ... is replaced"; the code needs the file unchanged on two 1 s checks, so it is one to two seconds.
