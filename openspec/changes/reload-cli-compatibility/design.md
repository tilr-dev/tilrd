# Design

## Context

At 13f9522 the supervisor's pre-exec check runs `<exe> state-formats` and compares the formats only (`supervisor.rs:1662-1681`); the successor gets the original argv (`supervisor.rs:1827-1829`). Flags added on the itests branch (`--socket`, `--lock-fd`, `--no-listen`) are unknown to `tilr-server` releases, and the configuration is read after the exec with `deny_unknown_fields` (`main.rs:281,298`, `supervisor.rs:363-400`).

## Decisions

### D1. Advertise flags in `state-formats`
`state-formats` prints, after the existing format lines, one `flags: --a --b ...` line and supports `state-formats --check-config PATH` (exit 0 / 1 with the reason on stderr). The supervisor parses its own argv for flag names (not values) and requires each to be listed. Alternative: run `<exe> --help` and grep — rejected, help text is not a contract.

### D2. Old successors
A successor whose `state-formats` has no `flags:` line is a release from before this change. The supervisor carries a table of the flags such releases know (those of the archive releases published before this change; a release older than 2a8b425 has no archive and cannot be updated to) and refuses only when argv uses another. This keeps a plain `tilrd` reload into an old release working and refuses the `tilr`-started case that kills shells today.

### D3. Signal mask
`sigprocmask(SIG_BLOCK, {USR1, HUP})` right before `execve`; the mask survives exec; the successor unblocks after its handlers are in place. A pending USR1 then triggers one reload in the successor.

## Reload and reconnect

Nothing new crosses the reload: no state file field, no new connection. Clients reconnect as on any reload. A refused reload keeps the old image and every connection.

## Risks

- The table of old flags must be right for every published release; verified by a UC-066-style rollback case against the previous archive release on the CDN.
