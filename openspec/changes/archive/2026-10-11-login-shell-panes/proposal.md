# Proposal: Panes run the user's login shell

> Retro archive entry, reconstructed on 2026-10-18 from git history, Basic Memory notes and itests/USE_CASES.md (branch `itests`). Spec deltas are not kept; the resulting behavior is in `openspec/specs/`.

## Why

Under launchd the Mac app had to wrap the server in zsh only to get a usable PATH, and macOS showed "zsh can run in background" as a Login Item (Basic Memory `main/projects/tilr/tilr-lokalnyi-server-pod-launchd-bez-zsh-obiortki-panel-login-shell`).

## What Changes

- A pane without a command starts the shell from the password database (then `$SHELL`, then `/bin/sh`) with `argv[0] = -<name>`, so the shell reads its own profile.
- The pane gets `SHELL`, `HOME`, `USER`, `LOGNAME`; launchd's `XPC_*` are dropped; `TILR_SHIMS` names the shims directory for a profile that rebuilds PATH.
- An explicit command runs as before.

## Capabilities

### Modified Capabilities
- `panes`

## Out of scope

- A ZDOTDIR wrapper for profiles that drop the shims (open change `shims-path-robustness`).

## Impact

Implemented by: [`437ff95`](https://github.com/tilr-dev/tilrd/commit/437ff95) Panes run the user's login shell … (2026-10-11).
