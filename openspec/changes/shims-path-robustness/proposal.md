# Proposal: Keep the shims first on PATH when a profile rebuilds it

## Why

A login shell may rebuild PATH from scratch (Debian's `/etc/profile`) and lose the shims directory; the workaround is `PATH="$TILR_SHIMS:$PATH"` in the user's profile; a ZDOTDIR-style wrapper is not done (Basic Memory `main/projects/tilr/tilr-lokalnyi-server-pod-launchd-bez-zsh-obiortki-panel-login-shell`; test note in f12d73d).

## What Changes

- To be designed: per-shell startup wrappers (zsh `ZDOTDIR`, bash `--rcfile`, fish `--init-command`) that source the user's files and then put `TILR_SHIMS` first.

## Capabilities

### Modified Capabilities
- `panes`

Spec deltas are not written yet (`skip_specs: true`).

## Out of scope

- Shells other than zsh, bash and fish.

## Impact

`crates/tilrd/src/pty.rs`; wrapper files in the runtime directory, recreated after a reload.
