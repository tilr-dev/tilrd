## MODIFIED Requirements

### Requirement: Exec is offered unless turned off
The server SHALL list `"exec"` in `HelloAck.features` unless it runs with `--no-exec` or the configuration file has `exec = false` (`--no-exec` wins; unset is on). With exec off, an `ExecStart` without `server_command` MUST be answered with `ExecStarted` carrying its `request_id`, `exec_id` 0 and `error` `exec is turned off`, and the connection MUST stay open. Exec takes the same access token as everything else.

**Verified by**: `exec::exec_is_offered_unless_turned_off` and the unit test `config::tests::exec_is_on_unless_turned_off` (crates/tilrd/src/config.rs).

#### Scenario: Turned off
- **WHEN** the server runs with `--no-exec` (or `exec = false`) and a client sends `ExecStart` for `true` and then `Ping`
- **THEN** `"exec"` is not in `HelloAck.features`, the client receives `ExecStarted` with `exec_id` 0 and error `exec is turned off`, and then `Pong`

### Requirement: Nothing of a program outlives its exec
When the program exits, whatever is left in its process group SHALL be killed with SIGKILL before the program is reaped, and the exec SHALL end with it, even when a leftover process held its stdout open. A pipe still held open by a process outside the group SHALL be read for at most 1 s of waiting on that pipe after the group is gone, however much that process writes (time the server waits for room on the client's bulk lane does not count), and then given up; `ExecCancel`, the time limit and the client going MUST end that wait at once. On Linux the program MUST also be killed should the server die without killing it (`PR_SET_PDEATHSIG`; its children then go only with the group).

**Verified by**: `exec::what_a_program_leaves_running_in_its_group_goes_when_it_exits` and `exec::a_process_outside_the_group_that_keeps_writing_does_not_hold_the_exec`.

#### Scenario: Background child holding stdout
- **WHEN** a client runs `sh -c 'sleep 50 & echo $! > file; echo bye'`
- **THEN** stdout is `bye\n`, code is 0, the exec ends within 5 s, and the `sleep 50` is gone

#### Scenario: Escaped writer
- **WHEN** a client runs `sh -c 'setsid sh -c "while :; do echo x; done" & echo bye'`
- **THEN** `ExecExit` with code 0 arrives within 3 s, and a 65th exec can then be started where 64 ran

#### Scenario: Cancel during the drain
- **WHEN** a client runs `sh -c 'setsid sh -c "while :; do echo x; sleep 0.01; done" &'` and sends `ExecCancel` as soon as the shell has exited
- **THEN** `ExecExit` with `cancelled` true arrives within 1 s

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

## ADDED Requirements

### Requirement: Exec replies are never silently lost
`ExecStarted` and `ExecInputAck` SHALL go on the client's interactive lane; when that lane cannot take one (it is full), the server MUST drop the client as too slow (log "client too slow", close code 4001, its execs killed) rather than lose the reply.

**Verified by**: unit test `exec::tests::a_reply_the_lane_cannot_take_drops_the_client` (crates/tilrd/src/exec.rs).

#### Scenario: Full lane at ExecStarted
- **WHEN** a client's interactive lane is full and it sends `ExecStart`
- **THEN** its connection is closed with code 4001 and no program is left running for it
