# Design

## Context

At 13f9522 signal handlers are reset by `execve` and the successor installs its own only after it has parsed its arguments and read its state, so SIGUSR1 or SIGHUP in that window kills the supervisor (SIGTERM too, without the clean shutdown). The flag and configuration check this change first planned (a `flags:` line in `state-formats`, `--check-config`, a table for older releases) is done by `reload-rollback`'s check process, which also refuses every successor that does not print `reload-check 1`.

## Decisions

### D1. Signal mask
The supervisor blocks SIGUSR1, SIGHUP and SIGTERM on the thread that execs, in the `pre_exec` hook of the exec (after anything the standard library does to the mask, right before `execve`); the mask survives the exec, and a signal arriving then stays pending. If the exec fails, the same thread unblocks them again.

Changed from the first draft ("the successor unblocks after its handlers are in place"): a successor that kept the mask until its tokio handlers are registered would start its runtime's threads with the signals blocked, and every child it spawns from them inherits that mask (the standard library no longer resets it in a child): pane processes, shells, execs and `cloudflared` would run with SIGHUP and SIGTERM blocked. So instead, first thing in `main`, while the process has one thread, an executable started with any of the three blocked installs a plain handler for them that only records which came, then unblocks them: a pending signal is recorded at once, and every thread and child after that has them unblocked. The supervisor registers its tokio handlers before spawning anything, then turns what was recorded into what the handlers would have done: SIGUSR1 one reload, SIGTERM a shutdown, SIGHUP the `SIGHUP ignored` log line. The recording handler stays installed underneath tokio's (signal-hook chains to it) and records nothing after that point. The window from the `execve` to the first lines of `main` is covered by the mask, the rest by the recording handler.

### D2. The socket after a refusal
A reload refused by `reload-rollback`'s check (or by `state-formats`), or failing before the exec, leaves the unix socket file, its flock lock and the accept loop untouched; nothing is unlinked or closed before the exec succeeds. The check runs before anything is stopped, so a refusal has nothing to undo. A server that serves its unix socket no longer reloads into an executable that does not print `unix-socket 1` (before: one with a TCP port did, serving TCP alone, and the socket file stayed behind with no server on it, so the next `tilr` started a second server): it logs `not reloading: <exe> cannot take over the Unix socket this server serves` and carries on.

## Reload and reconnect

Nothing new crosses the reload: no state file field, no new connection. Clients reconnect as on any reload. A refused reload keeps the old image, every connection and the socket with its lock.

## Risks

- A successor that does not unblock the signals would never get them. Every successor reloaded into prints `reload-check 1` (`reload-rollback` refuses the others) and both changes ship in the same release, so the mask is set for every successor. A build with `reload-rollback` and without this change (an intermediate commit on `dev`, never released) reloaded into from this one would keep them blocked.
- A SIGUSR1 that the old image takes while a reload is under way (during the checks) is part of that reload, as before; only the signals arriving from the exec on are held for the successor.
- The recording handler and tokio's both see a signal that comes in the instant between tokio's registration and reading the record: one extra reload or log line, never a lost signal.
