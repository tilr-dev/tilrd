# Design

## Context

At 47a8557 the supervisor runs `<exe> state-formats` (10 s limit), compares the formats, writes `server-<pid>.state`, execs in place with its original argv and then tells each pane process to reload; each pane writes `panes/<pane id>.state` and execs. Nothing between `state-formats` and the live resume tells whether the successor accepts the argv, the configuration and the state, so a panic in resume (failpoint `supervisor-resume=panic`) or a clap exit 2 ends the server and every shell (`reload::a_version_that_crashes_taking_over_ends_the_server_and_its_shells`).

## Decisions

### D1. The check child
After `state-formats` and after writing its state file, the supervisor copies the state file to `server-<pid>.check.state` (0600, same directory) and spawns `<exe>` with its exact argv and environment plus `TILR_RELOAD_CHECK=<fd>`, where `<fd>` is the write end of a pipe (inherited, not CLOEXEC). The child is a new process (fork + exec via `Command`), so the supervisor keeps its pid; the child gets no listener, socket, lock or pty descriptor (all CLOEXEC or closed for it).

### D2. What the child does
With `TILR_RELOAD_CHECK` set, `tilrd` parses its command line (clap), loads the configuration file as a live start would, reads and decodes the check state file (`supervisor-state` version and contents), runs the resume path up to the point where it would take over descriptors (it checks that every descriptor the state names is accounted for, without using it), then writes `ok\n` to the pipe and exits 0. Any error is written as `error: <reason>\n` and exits 1. It never binds, accepts, locks, starts panes, cloudflared or tasks.

### D3. The supervisor's verdict
It waits up to 10 s for a line on the pipe and the child's exit. `ok` and exit 0: remove the check file and exec as today. Anything else (error line, crash, signal, timeout, EOF): kill the child if still running, remove the check file and the state file, log `not reloading: the new executable could not resume: <reason>` and carry on. A reload that is refused is like "A reload that fails before the handover changes nothing".

### D4. Successors without the check
A successor whose `state-formats` lacks `reload-check 1` predates this change. It is reloaded into only if every flag in the supervisor's argv is in a table of the flags those releases accept (the archive releases published before this change); otherwise `not reloading: <exe> does not accept <flag>`. If no archive release predates this change when it lands, the table is empty and such successors are refused outright.

### D5. Pane processes
Before its exec a pane process does D1-D3 with `panes/<pane id>.check.state` and `TILR_RELOAD_CHECK` in pane mode (the pane's argv). On refusal it logs, removes its state file and carries on with its code, as "A pane that cannot hand over carries on".

### D6. Failpoints
`supervisor-resume` and `pane-resume` fire in the check child too (the child runs the resume path), so `supervisor-resume=panic` now gives a refused reload. New failpoints `supervisor-resume-live` and `pane-resume-live` fire only in the live resume after the exec, keeping the "crash after the confirmation" test.

## Reload and reconnect

The check files are in the runtime directory next to the state files, readable by the user only, and swept at start like stale state files. Nothing new crosses the reload; clients reconnect as on any reload; a refused reload keeps every connection.

## Risks

- A child that hangs costs up to 10 s of a reload (no client sees it: connections keep running).
- The check covers the resume up to the takeover; a crash after it is not covered (non-goal).
- D4's table must be right for every published release; verified by the UC-066 rollback to the previous archive release.
