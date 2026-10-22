# exec Specification

## Purpose

Exec (feature `"exec"`): a client runs a program on the server's machine without a pane or a shell (`ExecStart`), gets its stdout and stderr apart (`ExecOutput`) and how it ended (`ExecExit`), feeds its stdin (`ExecInput`, acknowledged by `ExecInputAck`), signals it (`ExecSignal`) or kills it (`ExecCancel`); and the server commands `tilr token` / `update` / `cloud` that `tilr` runs through it on the Unix socket. The bulk lane that carries the output is in the `protocol` spec; how `tilr` shows a server command's output is in the `tilr-cli` spec.

## Requirements

### Requirement: Exec is offered unless turned off
The server SHALL list `"exec"` in `HelloAck.features` unless it runs with `--no-exec` or the configuration file has `exec = false` (`--no-exec` wins; unset is on). With exec off, an `ExecStart` without `server_command` MUST be answered with `ExecStarted` carrying its `request_id`, `exec_id` 0 and `error` `exec is turned off`, and the connection MUST stay open. Exec takes the same access token as everything else.

**Verified by**: `exec::exec_is_offered_unless_turned_off` and the unit test `config::tests::exec_is_on_unless_turned_off` (crates/tilrd/src/config.rs).

#### Scenario: Turned off
- **WHEN** the server runs with `--no-exec` (or `exec = false`) and a client sends `ExecStart` for `true` and then `Ping`
- **THEN** `"exec"` is not in `HelloAck.features`, the client receives `ExecStarted` with `exec_id` 0 and error `exec is turned off`, and then `Pong`

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

### Requirement: ExecStarted tells the exec's id or why it did not start
Every `ExecStart` SHALL be answered with one `ExecStarted` echoing `request_id`, before any output: on success with `exec_id` (chosen by the server from 1, never reused while it runs) and `pid` (also the process group id); otherwise with `exec_id` 0 and `error` (`no program`, `<program>: <OS error>`, `no such directory: <dir>`, `not NAME=value: "<entry>"`, `64 execs running already`), and nothing else comes of it.

**Verified by**: `exec::a_program_runs_without_a_shell_and_its_outputs_come_apart`.

#### Scenario: Refusals
- **WHEN** a client starts `no-such-program-tt`, an empty `argv`, `true` with `cwd` `/no/such/dir`, and `true` with `env` `["NOEQUALS"]`
- **THEN** each `ExecStarted` has `exec_id` 0 and an error containing `no-such-program-tt`, `no program`, `no such directory`, respectively `NAME=value`
- **AND** the server's log has `exec refused`, `exec started` and `exec ended`

### Requirement: Execs are logged
The log SHALL have `exec started`, `exec ended` (with code, signal, why the server killed it and its duration) and `exec refused`, each with the client's `Hello.client_id`.

**Verified by**: `exec::a_program_runs_without_a_shell_and_its_outputs_come_apart`.

#### Scenario: Started, ended, refused
- **WHEN** a client runs a program and asks for one that does not exist
- **THEN** the server's log has `exec started`, `exec ended` and `exec refused`

### Requirement: Output comes apart per stream, then ExecExit
Stdout and stderr SHALL come as `ExecOutput` with `stream` 1 and 2, at most 64 KiB of `data` each, in order per stream, then one `ExecExit`, the exec's last message. `ExecExit` SHALL carry `code` (0 if a signal killed it), `signal` (0 if none), `timed_out`, `cancelled`, and `reason` (`"reload"`, `"shutdown"`, `"input overrun"`, or empty).

**Verified by**: `exec::a_program_runs_without_a_shell_and_its_outputs_come_apart` and UC-077 (`uc_077.py::exec_returns_stdout_and_stderr_apart_and_the_exit_code`).

#### Scenario: Exit code and both streams
- **WHEN** a client runs `sh -c 'echo out; echo err >&2; exit 3'`
- **THEN** stdout is `out\n`, stderr is `err\n`, and `ExecExit` has code 3, signal 0, `timed_out` and `cancelled` false and an empty `reason`

### Requirement: Output is paced by the client
`ExecOutput` and `ExecExit` SHALL go by the client's bulk lane (see the `protocol` spec): the program's pipes are read only as fast as the lane has room, so a client that reads slowly holds the program up, blocked writing, instead of the server buffering its output.

**Verified by**: `exec::a_large_output_arrives_whole_in_order_and_holds_up_a_slow_client` and UC-077 (`uc_077.py::a_large_output_arrives_whole_and_in_order`).

#### Scenario: 32 MiB to a client that waits
- **WHEN** a program writes 32 MiB to stdout, then `tail` to stderr, then touches a file, and the client reads nothing for 1 s
- **THEN** the file does not exist yet; once the client reads, stdout is the 32 MiB whole and in order, stderr is `tail\n` and code is 0

### Requirement: Stdin is fed within a 4 MiB window
With `stdin`, `ExecInput.data` SHALL be written to the program's stdin in order and `eof` SHALL close it after what is queued. `ExecInputAck.taken` SHALL tell the bytes taken in all as it takes them (written, or dropped once it closed stdin). A client MUST keep at most 4 MiB sent and not yet taken; more MUST kill the exec's group, `reason` `"input overrun"`.

**Verified by**: `exec::stdin_reaches_the_program_and_is_acknowledged` and UC-077 (`uc_077.py::stdin_given_to_exec_reaches_the_program`).

#### Scenario: 8 MiB within the window
- **WHEN** a client feeds 8 MiB to `wc -c` in 64 KiB pieces, never more than 4 MiB ahead of the last `ExecInputAck.taken`, then `eof`
- **THEN** stdout is `8388608`

#### Scenario: Empty and closed
- **WHEN** a client starts `wc -c` with `stdin` and sends `ExecInput` with no data and `eof`
- **THEN** stdout is `0`

#### Scenario: Overrun
- **WHEN** a client sends 5 MiB in 1 MiB pieces to `sleep 30` started with `stdin`
- **THEN** `ExecExit` has `reason` `"input overrun"` and signal 9, and the program is gone

### Requirement: Input that is not wanted is ignored
`ExecInput` after `eof`, for an exec started without `stdin`, or for an exec that is not the client's MUST be ignored, the exec going on.

**Verified by**: `exec::input_that_is_not_wanted_is_ignored`.

#### Scenario: Input after eof
- **WHEN** a client sends `ExecInput` data after one with `eof`
- **THEN** the data is dropped and the program sees end of file as before

### Requirement: The program leads its own process group, which ExecSignal reaches
The program SHALL lead a process group of its own (pgid = `ExecStarted.pid`). `ExecSignal` SHALL send its `signal` to that group, as a terminal does for Ctrl+C, unanswered; one for an exec not the client's, or being killed, is ignored. A program killed by a signal SHALL report it in `ExecExit.signal`, code 0.

**Verified by**: `exec::the_signal_that_killed_a_program_is_reported` and UC-077 (`uc_077.py::exec_reports_the_signal_that_killed_the_program`).

#### Scenario: Killed by its own SIGTERM
- **WHEN** a client runs `sh -c 'kill -TERM $$'`
- **THEN** `ExecExit` has code 0 and signal 15

#### Scenario: SIGINT from the client
- **WHEN** a client sends `ExecSignal{signal: 2}` to a running `sleep 30`
- **THEN** `ExecExit` has signal 2 and `cancelled` false

### Requirement: A time limit kills the program and its group
With `timeout_ms` greater than 0 the server SHALL kill the program's whole process group with SIGKILL once that time has passed and report `ExecExit.timed_out` true with signal 9; 0 means no limit.

**Verified by**: `exec::a_time_limit_kills_the_program_and_its_children` and UC-077 (`uc_077.py::exec_with_a_time_limit_is_killed_and_leaves_nothing_behind`).

#### Scenario: Half a second
- **WHEN** a client runs a script that starts a background `sleep 30` and then `sleep 31`, with `timeout_ms` 500
- **THEN** `ExecExit` has `timed_out` true and signal 9 within 3 s, and neither the program nor its background child is alive

### Requirement: ExecCancel kills at once
`ExecCancel` SHALL kill the program's whole process group with SIGKILL at once and report `ExecExit.cancelled` true (with `timed_out` false and signal 9).

**Verified by**: `exec::a_cancelled_program_ends_at_once_with_its_children` and UC-077 (`uc_077.py::a_cancelled_exec_ends_at_once_with_its_children`).

#### Scenario: Cancel with a child running
- **WHEN** a program has a background child running and the client sends `ExecCancel`
- **THEN** `ExecExit` has `cancelled` true and signal 9, and the program and its child are gone

### Requirement: Nothing of a program outlives its exec
When the program exits, what is left in its process group SHALL be killed with SIGKILL before the program is reaped, and the exec SHALL end with it. A pipe held open by a process outside the group SHALL be read after the group is gone for at most 1 s of waiting on it (not counting waits for room on the bulk lane) and 1 MiB, then given up; `ExecCancel`, the time limit and the client going MUST end that at once. On Linux the program MUST also die with the server (`PR_SET_PDEATHSIG`).

**Verified by**: `exec::what_a_program_leaves_running_in_its_group_goes_when_it_exits`, `exec::a_process_outside_the_group_that_keeps_writing_does_not_hold_the_exec`, `exec::a_cancel_during_the_drain_ends_the_exec_at_once` and the unit test `exec::tests::the_group_is_killed_while_the_program_is_a_zombie` (crates/tilrd/src/exec.rs).

#### Scenario: Background child holding stdout
- **WHEN** a client runs `sh -c 'sleep 50 & echo $! > file; echo bye'`
- **THEN** stdout is `bye\n`, code is 0, the exec ends within 5 s, and the `sleep 50` is gone

#### Scenario: Escaped writer
- **WHEN** a client runs `sh -c 'setsid yes & echo bye'` (the shell going on once `yes` has left its group)
- **THEN** `ExecExit` with code 0 arrives within 3 s, and a 65th exec can then be started where 64 ran

#### Scenario: Cancel during the drain
- **WHEN** a client runs `sh -c 'setsid sh -c "while :; do echo x; sleep 0.01; done" &'` and sends `ExecCancel` as soon as the shell has exited
- **THEN** `ExecExit` with `cancelled` true arrives within 1 s

### Requirement: A client's execs end with its connection
When a client's connection ends (closed, dropped, superseded), the server SHALL kill the process groups of all its execs with SIGKILL and log `client gone`.

**Verified by**: `exec::a_client_that_goes_takes_its_programs_with_it` and UC-077 (`uc_077.py::a_client_that_goes_takes_its_execs_and_their_children_with_it`).

#### Scenario: Client drops
- **WHEN** a client with a running exec that has a background child disconnects
- **THEN** the program and its child are gone and the log has `client gone`

### Requirement: Execs do not survive a reload or a stop
Execs SHALL NOT be handed over in a reload: the server MUST kill every exec's group (SIGKILL) and, before the connections go, send each connected client the output so far and `ExecExit` `reason` `"reload"` (`"shutdown"` on a stop), waiting up to 1 s. For an exec that has not reported in that time the server MUST stop reading its output first and then queue an `ExecExit` with only `exec_id` and `reason` behind the output already queued, so no `ExecOutput` of that exec follows it.

**Verified by**: `exec::a_reload_ends_the_programs_and_their_clients_hear_why`, `exec::an_exec_given_up_at_a_reload_ends_with_its_exec_exit` and UC-077 (`uc_077.py::a_reload_ends_the_running_execs_with_their_children_and_says_why`).

#### Scenario: Reload with an exec running
- **WHEN** an exec has printed `started` and has a background child, and the server gets SIGUSR1
- **THEN** the client gets stdout `started\n` and `ExecExit` with `reason` `"reload"` and signal 9, the program and its child are gone
- **AND** after reconnecting, a new exec `echo again` runs and prints `again\n`

#### Scenario: An exec that does not report in time
- **WHEN** an exec whose output a client is not reading (bulk lane full) is running and the server gets SIGUSR1
- **THEN** once the client reads again, the last message of that exec it receives is `ExecExit` with `reason` `"reload"`

### Requirement: Execs run side by side
A client SHALL be able to run up to 64 execs at once, each running concurrently with its own output and exit, without holding up the panes.

**Verified by**: `exec::execs_run_side_by_side_with_the_panes` and UC-077 (`uc_077.py::eight_execs_run_at_once`).

#### Scenario: Eight one-second programs
- **WHEN** a client starts eight `sh -c 'sleep 1; echo run N'` at once
- **THEN** all eight end within 4 s, each exec's output is its own `run N\n`, and a pane then runs `true` with status 0

### Requirement: Server commands run on the Unix socket only
An `ExecStart` with `server_command` SHALL run the server's own executable with `argv` as its arguments (`token`, `update`, `cloud ...`), as the server's child in its environment, under the rules of any exec, whether or not exec is on. It MUST be taken only on the Unix socket; over TCP it MUST be refused with `ExecStarted.error` `server commands are taken on the Unix socket only`.

**Verified by**: `local::server_commands_run_for_local_clients_only` and `client::server_commands_run_in_the_server`.

#### Scenario: tilr token with exec off
- **WHEN** a server runs with `--no-exec` and a client on its Unix socket sends `ExecStart{argv: ["token"], server_command: true}`
- **THEN** `ExecStarted.error` is empty, the `ExecOutput` is the content of `state/token`, and `ExecExit.code` is 0

#### Scenario: Over TCP
- **WHEN** a client connected over TCP with the token sends the same `ExecStart`
- **THEN** `ExecStarted.error` contains `Unix socket only`

### Requirement: Exec replies are never silently lost
`ExecStarted` and `ExecInputAck` SHALL go on the client's interactive lane; when that lane cannot take one (it is full), the server MUST drop the client as too slow (log "client too slow", close code 4001, its execs killed) rather than lose the reply.

**Verified by**: unit test `exec::tests::a_reply_the_lane_cannot_take_drops_the_client` (crates/tilrd/src/exec.rs).

#### Scenario: Full lane at ExecStarted
- **WHEN** a client's interactive lane is full and it sends `ExecStart`
- **THEN** its connection is closed with code 4001 and no program is left running for it

## Notes

- Untested details inside requirements: the 64-exec-per-client refusal, `reason` `"shutdown"` on a server stop, `PR_SET_PDEATHSIG`, and ignoring a signal for an exec that is not the client's.
- UC-077 was red before 2026-10-17 (no Exec) and passes at HEAD per itests/USE_CASES.md; no case cited here is currently failing.
- Unit tests in `crates/tilrd/src/*.rs` are cited as `module::tests::name` with their file.
