## ADDED Requirements

### Requirement: The time limit holds when a descendant keeps the output open
A time limit and `ExecCancel` SHALL end an exec and produce its `ExecExit` even when a process outside the program's group still holds its stdout or stderr open.

#### Scenario: Escaped descendant
- **WHEN** an exec runs `sh -c 'setsid sleep 60 & sleep 60'` with a 1 s time limit
- **THEN** `ExecExit` arrives within 3 s with the time-limit reason

### Requirement: ExecExit is the last message of an exec
The server SHALL send no `ExecOutput` for an exec after its `ExecExit`.

#### Scenario: Output racing the exit
- **WHEN** a program writes a large output and exits at once
- **THEN** all `ExecOutput` messages of that exec precede its `ExecExit`

### Requirement: ExecStart is refused when exec is off
A server running with exec off SHALL answer `ExecStart` with an `ExecStarted` carrying an error instead of ignoring it.

#### Scenario: Exec off
- **WHEN** a client sends `ExecStart` to a server started with `--no-exec`
- **THEN** it receives `ExecStarted` with an error for that request
