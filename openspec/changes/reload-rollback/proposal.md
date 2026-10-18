# Proposal: Survive a successor that crashes after the exec

Блокер: How should a reload survive a successor that crashes after the exec? Options: (a) pre-exec dry run: the new binary reads a copy of the state and exits 0 — cheap, catches format/config crashes, not later ones; (b) child confirms then exec — same pid, catches crashes during resume; (c) separate successor process — survives any crash but the pid changes (breaks lock, MAINPID, kill -USR1). Recommendation: (a) now (with `reload-cli-compatibility`), (b) later if crashes during resume are seen.

Зависит от: reload-cli-compatibility

## Why

Basic Memory `main/projects/tilr/tilr-ustanovka-i-obnovlenie-install.sh-relizy-na-r2-tilr-update` (failpoints work, 2026-10-04): if the new version crashes after the exec, the server and every shell are lost; the only protection is the pre-exec `state-formats` check. Option recorded: run the successor as a separate process that confirms readiness before the old one hands over. "Решение за Dan". `core/TODO.md` also notes a pid-reuse weakness (a successor that died before reading its state file, the same pid reused) (Basic Memory `main/projects/tilr/tilr-core-id-panelei-uuid-tilr-server-ps-i-peredacha-sostoianiia-pri-perezagruzke`).

## What Changes

- To be decided: (a) keep exec in place and add a pre-exec dry run of the successor reading a copy of the state; (b) start the successor as a child that reads the state and confirms, then the old image execs into it; (c) successor as a separate process that takes over the listeners and pane connections (pid changes).

## Capabilities

### Modified Capabilities
- `reload`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Rollback of a bad release on the CDN (that is `tilrd update`'s fallback).

## Impact

`crates/tilrd/src/{supervisor,reload}.rs`, the pane reload path. Option (c) changes the pid across a reload, which `tilr`'s lock file, service managers (`Type=notify` MAINPID) and `kill -USR1` users rely on: **BREAKING** for them. Must survive an update from the previous release (the old image does the first reload).
