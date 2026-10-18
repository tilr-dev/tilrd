# Proposal: A reload checks that the successor accepts the command line and the configuration

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found that a reload execs the successor with the same arguments while the pre-exec check compares only state formats (findings #2, #7, #8, #19). A rollback to an old `tilr-server` with the same formats rejects `--socket` / `--lock-fd` (passed by `tilr`'s daemon start and the service unit) with clap exit 2 after the exec, and every shell dies (the rollback path of UC-066, against a release that predates a flag; since 2a8b425 only archive releases can be rolled back to, so an old `tilr-server` is out of reach). A configuration the successor refuses (an unknown key under `deny_unknown_fields`, `exec = false` with an older binary) is fatal after the exec. Signal handlers reset across `execve`, so a SIGUSR1 (or a SIGHUP, noted in Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia` as "SIGHUP in the first milliseconds after exec still kills the server") in that window kills the supervisor. A reload into a binary that cannot take the socket leaves the socket file, and the next `tilr` starts a second server.

## What Changes

- `tilrd state-formats` also lists the command-line flags this executable accepts (and a configuration-check entry point); before the exec the supervisor refuses a successor that does not accept every flag it was started with, or whose configuration check fails, logging why and keeping the old image.
- A successor that does not print the new lines (any release before this change) is reloaded into only when the command line uses no flag added after that release (`--socket`, `--lock-fd`, `--no-listen`), otherwise refused.
- SIGUSR1 and SIGHUP are blocked from just before the exec until the successor has installed its handlers.
- A refused or failed reload leaves the socket file and its lock as they were and serving; nothing is left that makes `tilr` start a second server.

## Capabilities

### Modified Capabilities
- `reload`: see `specs/reload/spec.md`.
- `daemon-lifecycle`: no requirement change.

## Out of scope

- Surviving a successor that crashes after the exec (change `reload-rollback`).
- Changing the CDN layout or `tilrd update`'s rollback fallback.

## Impact

`crates/tilrd/src/reload.rs` (`state-formats` output), `supervisor.rs` (pre-exec check, signal mask), `main.rs` (config check entry point). The `state-formats` output is a contract between releases: new lines are added after the existing ones so an older reader still finds its formats (not breaking). Has to work on a reload from the previous release and on a rollback to it; no state file format change, no PROTOCOL_VERSION change.
