# Proposal: A reload execs only into a successor that has shown it can resume

## Why

Basic Memory `main/projects/tilr/tilr-ustanovka-i-obnovlenie-install.sh-relizy-na-r2-tilr-update` (failpoints work, 2026-10-04): if the new version crashes after the exec, the server and every shell are lost; the only protection is the pre-exec `state-formats` check. `core/TODO.md` also notes a pid-reuse weakness (a successor that died before reading its state file, the same pid reused) (Basic Memory `main/projects/tilr/tilr-core-id-panelei-uuid-tilr-server-ps-i-peredacha-sostoianiia-pri-perezagruzke`). The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (findings #2, #7, #8, #19): a successor that rejects the command line (`--socket`, `--lock-fd`) or the configuration (`deny_unknown_fields`) fails after the exec and every shell dies.

## Decision (2026-10-18)

- Fork, confirm, then exec (same pid): before its exec the supervisor starts the new executable in a check mode on a copy of the handover state, with the command line and configuration it will get; only when that child confirms over a pipe does the supervisor exec in place. Pane processes do the same before their own exec.
- This check also covers what `reload-signals-and-socket` planned as its pre-exec flag and configuration check; that change keeps the signal mask and the socket after a refused reload.
- Rejected: a dry run that only reads the state (misses crashes in the resume path and on the command line).
- Rejected: a separate successor process (the pid changes, which breaks the flock lock, `MAINPID` and `kill -USR1`).

## What Changes

- `tilrd state-formats` prints one more line, `reload-check 1`, after the existing ones.
- Before the exec, when the successor prints `reload-check 1`, the supervisor runs it in check mode (design D1-D3), 10 s limit; on a confirmation it execs, otherwise it logs `not reloading: the new executable could not resume: <reason>`, removes the check files and carries on with every connection and pane.
- A pane process checks its successor the same way before its exec; a pane whose check fails carries on with its code (as "A pane that cannot hand over carries on").
- A successor without `reload-check 1` (a release before this change) is checked as today plus the table of command-line flags such releases accept (design D4).
- A crash after the confirmation (in the live resume) still ends the server and its shells, as today.

## Capabilities

### Modified Capabilities
- `reload`: see `specs/reload/spec.md`.

## Non-goals

- Surviving a crash after the confirmation, in the live resume (the server and its shells still end).
- Rollback of a bad release on the CDN (that is `tilrd update`'s fallback).
- Signals during the exec and the socket after a refused reload (change `reload-signals-and-socket`).

## Impact

`crates/tilrd/src/{main,supervisor,reload,pane}.rs`, the pane reload path, failpoints (`fail.rs`). No PROTOCOL_VERSION change, no state file format change: the check reads the same format. `state-formats` gains a line after the existing ones, so an older reader still finds its formats (not breaking). The first reload from the previous release is done by the old image, without the check; from then on every reload is checked. A rollback to a release before this change uses D4.
