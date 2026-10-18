# Proposal: Signals across the exec and the socket after a refused reload

Зависит от: reload-rollback

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) found that a reload execs the successor with the same arguments while the pre-exec check compares only state formats (findings #2, #7, #8, #19). Signal handlers reset across `execve`, so a SIGUSR1 (or a SIGHUP, noted in Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia` as "SIGHUP in the first milliseconds after exec still kills the server") in that window kills the supervisor. A reload into a binary that cannot take the socket leaves the socket file, and the next `tilr` starts a second server.

## Decision (2026-10-18)

- The command-line and configuration check moves to `reload-rollback`, whose check process runs the successor with the real argv and configuration (owner's decision there: fork, confirm, then exec).
- This change keeps what the check cannot cover: signals during the exec itself and the socket after a refused reload. It lands after `reload-rollback`, whose refusals its socket test uses.
- Rejected: a flag list in `state-formats` plus `--check-config` (the check process covers both with the real parser).

## What Changes

- SIGUSR1, SIGHUP and SIGTERM are blocked from just before the exec; the successor records what came while they were held and unblocks them first thing (design D1), then handles them once its handlers are in place: a SIGUSR1 in that window becomes one reload, a SIGHUP is ignored, a SIGTERM a clean shutdown. Children never inherit them blocked.
- A refused or failed reload leaves the socket file and its lock as they were and serving; nothing is left that makes `tilr` start a second server.
- A server that serves its unix socket does not reload into an executable that cannot take it (no `unix-socket 1`), with or without a TCP port (before: with a port it reloaded and left the socket file behind, finding #19).

## Capabilities

### Modified Capabilities
- `reload`: see `specs/reload/spec.md`.

## Non-goals

- Checking the successor's command line, configuration or resume (change `reload-rollback`).
- Surviving a successor that crashes after the exec.
- Changing the CDN layout or `tilrd update`'s rollback fallback.

## Impact

`crates/tilrd/src/supervisor.rs` (signal mask, socket rule), `reload.rs` (holding and recording the signals), the start of `main.rs`. No state file format, `state-formats` or PROTOCOL_VERSION change. A reload from the previous release works (the old image does not block signals; that first reload keeps today's window); a rollback to a release before `reload-rollback` is refused by it. Nothing new crosses a reload. Not breaking: the only executables that no longer get a TCP-only reload are ones that print `reload-check 1` but not `unix-socket 1`, and none exists.
