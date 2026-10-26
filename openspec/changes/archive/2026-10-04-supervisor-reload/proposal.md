# Proposal: The supervisor reloads itself in place; auto-reload on a replaced executable

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Pane processes could already reload in place, but a new supervisor still meant a restart. The same exec-in-place trick keeps the pid, keeps the pane processes as children and keeps the listening socket and pipes open as descriptors, so SCM_RIGHTS is not needed (Basic Memory `main/projects/tilr/tilr-zhurnal-reshenii`, `main/projects/tilr/tilr-ustanovka-i-obnovlenie-install.sh-relizy-na-r2-tilr-update`).

## What Changes

- On SIGUSR1, and when the executable on disk is replaced (polled once a second, identical on two checks), the supervisor execs the executable in place, then reloads the panes; `--no-auto-reload` turns the watch off.
- Supervisor state goes to the successor as protobuf `SupervisorState` in an unlinked file (`--resume <fd>` at the time); reading from panes stops between packets and the input queue is flushed; a pane that takes no input for 5 s is closed.
- The new executable is run with `--version` first; an error means no reload.
- The executable path is remembered at start, because on Linux `current_exe()` goes stale once the file is replaced.
- Client connections are dropped and clients reconnect.

## Capabilities

### Modified Capabilities
- `reload`

## Out of scope

- Rollback when the new version crashes after the exec (open change `reload-rollback`).
- Noticing a swapped symlink.

## Impact

Implemented by: [`58dcaea`](https://github.com/tilr-dev/tilrd/commit/58dcaea) Shared clipboard for image paste; supervisor reloads itself (2026-10-04).
