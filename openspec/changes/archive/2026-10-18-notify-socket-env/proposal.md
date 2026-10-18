# Proposal: Service-manager variables do not leak into panes and execs

## Why

The code review of branch `itests` (79397e2..13f9522, 2026-10-18) (finding #21): `NOTIFY_SOCKET`, set by systemd for a `Type=notify` unit (`tilr service install` writes one, `crates/tilr/src/service.rs:130`), is passed on to pane shells and execs. Re-checked at 1163abd: a pane's program gets the server's environment with only `XPC_SERVICE_NAME` and `XPC_FLAGS` removed (`crates/tilrd/src/pty.rs:93`, `210-212`); an exec gets the server's environment untouched (`crates/tilrd/src/exec.rs:410-430`). `tilr` already removes `NOTIFY_SOCKET` when it starts `tilrd` by hand (`crates/tilr/src/daemon.rs:228`), so only a server started by systemd is affected. With systemd's default `NotifyAccess=main` a message from a pane's program is ignored, so the effect is a stray variable that sd_notify-aware programs in a pane (and socket-activation code reading `LISTEN_FDS`) act on wrongly, not a takeover of the unit.

## What Changes

- The environment of a pane's program and of an exec lacks `NOTIFY_SOCKET`, `LISTEN_FDS`, `LISTEN_FDNAMES`, `LISTEN_PID`, `WATCHDOG_USEC` and `WATCHDOG_PID`, removed with the `XPC_*` ones; an `ExecStart` may still set any of them in its `env`.
- The server's own environment is unchanged: it still says `READY=1` after a reload.

## Capabilities

### Modified Capabilities
- `panes`: see `specs/panes/spec.md`.
- `exec`: see `specs/exec/spec.md`.

## Non-goals

- Changing the unit `tilr service install` writes (`Type=notify`, default `NotifyAccess`).
- Scrubbing other inherited variables.

## Impact

`crates/tilrd/src/pty.rs` (the removed-variables list, shared), `crates/tilrd/src/exec.rs` (remove before the `ExecStart` env is applied). Holds after a reload: the successor builds pane and exec environments the same way, and panes started before an update keep the environment they had. No protocol change, no state change.

Implemented by: [`4e7812b`](https://github.com/tilr-dev/tilrd/commit/4e7812b) fix: keep the service manager's variables out of panes and execs (2026-10-18).
