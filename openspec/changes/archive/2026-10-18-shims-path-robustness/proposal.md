# Proposal: Keep the shims first on PATH when a profile rebuilds it

## Why

- A pane without a command runs the user's login shell (`crates/tilrd/src/pty.rs:171-196` at 1163abd, `-<name>` as `argv[0]`). The server puts the shim directory (`xclip`, `wl-paste`, `tilr` linked to tilrd, `<runtime dir>/bin`) first on the panes' `PATH` and names it in `TILR_SHIMS` (`crates/tilrd/src/supervisor.rs:729-778`), but only in the environment the shell starts with.
- A login profile may set `PATH` from scratch (Debian's `/etc/profile`, a `PATH=...` without `$PATH` in `~/.profile`) and lose the directory; the documented workaround is `PATH="$TILR_SHIMS:$PATH"` in the user's profile (`crates/tilrd/src/clipboard.rs:30-34`, spec `clipboard-host-events` "The panes get the server's xclip and wl-paste first on PATH"). A ZDOTDIR-style wrapper was considered and not done (Basic Memory `main/projects/tilr/tilr-lokalnyi-server-pod-launchd-bez-zsh-obiortki-panel-login-shell`; test note in f12d73d).
- Without the shims a pasted image does not reach Claude Code (`xclip`) and `tilr edit` / `tilr open` in a pane run the user's own `tilr` instead of the pane's.
- The `panes` scenario "The tools survive the login profile" checks only that `$TILR_SHIMS` is somewhere on `PATH`, with a profile that does not reset it.

## What Changes

- For a pane running the user's login shell when that shell is zsh, bash or fish, and the server has tools for the panes, the server starts the shell through a small startup wrapper of its own (design D1-D3) that runs the user's startup files as that login shell would and then puts `$TILR_SHIMS` first on `PATH` (once, other copies removed).
- The wrappers are written by the supervisor next to the shims, in `<runtime dir>/shell/` (0700), at every start and after every reload.
- Other shells, panes with a command, and a server whose wrappers cannot be written behave as today (the latter logs `shell startup wrappers are off: <reason>`).
- The `TILR_SHIMS` hint stays for other shells.

## Capabilities

### Modified Capabilities
- `panes`: see `specs/panes/spec.md`.

## Non-goals

- Shells other than zsh, bash and fish; panes with a command.
- Shell integration beyond `PATH` (prompt marks, cwd reporting, titles).
- A switch to turn the wrappers off (a user who wants another order sets `PATH` in an interactive hook of their own, which runs after the wrapper's step for bash and fish; for zsh see design D1).

## Impact

`crates/tilrd/src/pty.rs` (how a login shell is started), `crates/tilrd/src/supervisor.rs` / `clipboard.rs` (writing the wrappers with the shims), README pane environment, itests runtime image (zsh, fish) and a new case, the `panes` spec. Not breaking; no protocol, PROTOCOL_VERSION or feature change. Survives a reload with nothing handed over: running shells already read their startup files; the successor rewrites the wrappers at its start for panes made after it. An update from the previous release changes only panes made after it.

Implemented by: [`0590786`](https://github.com/tilr-dev/tilrd/commit/0590786) fix(pane): keep the pane tools first on PATH after the login profile (2026-10-18).
