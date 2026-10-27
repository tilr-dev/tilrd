# Design

## Context

At 47a8557 the supervisor runs `<exe> state-formats` (10 s limit), compares the formats, writes `server-<pid>.state`, execs in place with its original argv and then tells each pane process to reload; each pane writes `panes/<pane id>.state` and execs. Nothing between `state-formats` and the live resume tells whether the successor accepts the argv, the configuration and the state, so a panic in resume (failpoint `supervisor-resume=panic`) or a clap exit 2 ends the server and every shell (`reload::a_version_that_crashes_taking_over_ends_the_server_and_its_shells`).

## Decisions

### D1. The check child
After `state-formats` and before it stops anything for the reload (execs, pane readers, `cloudflared`), the supervisor builds the state it would hand over from its panes as they are (the same function the handover uses, with the pipes' descriptor numbers and no unparsed bytes), writes it to `server-<pid>.check.state` (0600, same directory, written whole like a state file) and spawns `<exe>` with its exact argv and environment plus `TILR_RELOAD_CHECK=<path of the check file>`, stdin `/dev/null`, stdout and stderr piped. The child is a new process (`Command`), so the supervisor keeps its pid; the child gets no listener, socket, lock or pty descriptor (all CLOEXEC).

Changed from the first draft (an inherited pipe descriptor named by `TILR_RELOAD_CHECK=<fd>`, and a copy of the state file written for the exec): the child has another pid, so it cannot find `server-<pid>.state` by its own and is told the path; its stdout is the answer channel, so no descriptor has to be made inheritable; and a check made before anything is stopped leaves a refusal with nothing to undo (execs and the tunnel keep running).

### D2. What the child does
With `TILR_RELOAD_CHECK` set, `tilrd` logs nothing, parses its command line (clap, `try_parse`: a rejected argument is answered, not printed as usage), loads the configuration file as a live start would, reads and decodes the check file (format version included) and runs the resume path up to the point where it would take over descriptors: the failpoint `supervisor-resume`, a listening socket handed over, every descriptor the state names non-negative and named once, every pane with a valid id and a pid, the access token readable. A pane process (`pane --id ...`) decodes its check file, passes `pane-resume` / `pane-resume:<id>` and restores the screen from the snapshot. Then it writes `ok` to stdout and exits 0; any error is one line `error: <reason>` and exit 1; a panic hook answers a panic as `error: panicked at <file>:<line>: <message>`. It never binds, accepts, locks, starts panes, `cloudflared` or tasks, and does not remove the check file.

### D3. The supervisor's verdict
It waits up to 10 s for the child's exit and output, while its event loop goes on serving clients and panes (a reload asked for meanwhile is the one under way; a shutdown drops the wait and kills the child). `ok` and exit 0: exec as today, unless a pane started meanwhile (then the reload is tried again 200 ms later, check included). Anything else (an `error:` line, a crash, a signal, `ok` with a failed exit, EOF, timeout): kill the child if still running, remove the check file, log `not reloading: the new executable could not resume: <reason>` and carry on. The reason is the child's `error:` text, or its exit status and the last three non-empty lines of its stderr (400 characters at most). A refused reload is like "A reload that fails before the handover changes nothing".

### D4. Successors without the check
A successor whose `state-formats` lacks `reload-check 1` predates this change and is not reloaded into (`not reloading: <exe> cannot take over: it cannot show that it can resume (no `reload-check 1`)`). Owner's decision (2026-10-18): no compatibility with releases before this change; no archive-format release predates it. Started for a check, such an executable would ignore `TILR_RELOAD_CHECK` and run as a server or pane of its own, so it is never started that way. The table of flags older releases accept (first draft) is dropped.

### D5. Pane processes
After taking its screen snapshot and before writing its state file, a pane process does D1-D3 with `panes/<pane id>.check.state` and its own argv (`pane --id ...`); the pane is not served for the few milliseconds the check takes (up to 10 s for a hanging one), its pty buffers what the shell writes. On refusal it logs `reload failed: the new executable could not resume: <reason>`, writes no state file, restores its screen and carries on with its code, as "A pane that cannot hand over carries on". A pane does not run `state-formats` itself: the supervisor that asked it to reload has just checked the same file.

### D6. Failpoints
`supervisor-resume` and `pane-resume` (`pane-resume:<id>`) fire in the check child (the child runs the resume path), so `supervisor-resume=panic` now gives a refused reload. New failpoints `supervisor-resume-live` and `pane-resume-live` (`pane-resume-live:<id>`) fire only in the live resume after the exec, keeping the "crash after the confirmation" tests.

## Reload and reconnect

The check files are in the runtime directory next to the state files, readable by the user only, removed by the process that wrote them whatever the verdict, and swept at start like stale state files (`server-*.state`, `panes/*.state`, which they match). Nothing new crosses the reload; clients reconnect as on any reload; a refused reload keeps every connection, exec and stream. No data is queued for clients by the check, so the outbox and its backpressure are unchanged.

## Risks

- A child that hangs costs up to 10 s of a reload; clients and panes are served meanwhile (the supervisor's loop runs), a pane process waits for its own check.
- The check covers the resume up to the takeover; a crash after it is not covered (non-goal).
- The file is checked, then exec'd by path: a file replaced again between the two is exec'd unchecked (the automatic reload waits for the file to look the same on two checks a second apart, which narrows this to a deliberate second replacement within milliseconds).
- The check child shows in `ps` for a moment with the server's command line (and `TILR_RELOAD_CHECK` in its environment).
- Each reload starts one more process per pane (its check), next to the exec it already does.
