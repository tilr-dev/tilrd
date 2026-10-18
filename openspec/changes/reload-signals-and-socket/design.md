# Design

## Context

At 13f9522 signal handlers are reset by `execve` and the successor installs its own only after it has parsed its arguments and read its state, so SIGUSR1 or SIGHUP in that window kills the supervisor. The flag and configuration check this change first planned (a `flags:` line in `state-formats`, `--check-config`, a table for older releases) is done by `reload-rollback`'s check process.

## Decisions

### D1. Signal mask
`sigprocmask(SIG_BLOCK, {USR1, HUP})` right before `execve`; the mask survives exec; the successor unblocks after its handlers are in place. A pending USR1 then triggers one reload in the successor; a pending HUP is handled as the server handles HUP (ignored).

### D2. The socket after a refusal
A reload refused by `reload-rollback`'s check, or failing before the exec, leaves the unix socket file, its flock lock and the accept loop untouched; nothing is unlinked before the exec succeeds.

## Reload and reconnect

Nothing new crosses the reload: no state file field, no new connection. Clients reconnect as on any reload. A refused reload keeps the old image and every connection.

## Risks

- A successor from before this change would start with USR1 and HUP blocked and never unblock them. So the supervisor sets the mask only for a successor that prints `reload-check 1` in `state-formats`, and a successor that prints it unblocks after installing its handlers (both land with or after `reload-rollback`).
