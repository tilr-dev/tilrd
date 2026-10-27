# Design

## Context

At 1163abd a login-shell pane is `Command::new(shell)` with `arg0("-<name>")` and the shims in the environment (`pty.rs:171-196`, `supervisor.rs:729-778`). The startup files run after the environment is set, so a file that assigns `PATH` wholesale drops the shims. Terminals solve the same problem for their shell integration with per-shell wrappers (ghostty and kitty: zsh via `ZDOTDIR`, bash via `--posix` + `ENV`, fish via `--init-command`); this design follows them, limited to `PATH`.

## Decisions

### D1. zsh: ZDOTDIR and a one-shot precmd hook
The pane gets `ZDOTDIR=<runtime dir>/shell/zsh` (the server's own `ZDOTDIR`, if any, saved in `TILR_ZDOTDIR`). That directory's `.zshenv` restores `ZDOTDIR` (from `TILR_ZDOTDIR`, else unsets it), unsets `TILR_ZDOTDIR`, sources the user's `${ZDOTDIR:-$HOME}/.zshenv` if present, and adds a `precmd` function that puts `$TILR_SHIMS` first on `path` and removes itself. zsh then reads the user's `.zprofile`, `.zshrc`, `.zlogin` from the restored `ZDOTDIR` as usual; the hook runs before the first prompt, after all of them. `argv[0]` stays `-zsh`.

### D2. bash: posix mode and ENV
The pane runs `bash --posix` with `argv[0]` `-bash` and `ENV=<runtime dir>/shell/bash/env.sh`. In posix mode bash reads only `$ENV`; the file unsets `ENV`, runs `set +o posix`, sources `/etc/profile` and then the first readable of `~/.bash_profile`, `~/.bash_login`, `~/.profile` (bash's login order), and last puts `$TILR_SHIMS` first on `PATH`. `$0` is `-bash` and `shopt login_shell` is on, as before.

### D3. fish: --init-command
The pane runs `fish --login --init-command '<fix>'` with `argv[0]` `-fish`; fish evaluates the command after its configuration, so `<fix>` (`if set -q TILR_SHIMS; set -gx PATH $TILR_SHIMS (string match -v -- $TILR_SHIMS $PATH); end`) runs last.

### D4. Which shell is which
By the file name of the login shell (`zsh`, `bash`, `fish`, after a leading `-`); anything else is started as today. A shell given as a pane command is never wrapped.

### D5. Where the wrappers live
The supervisor writes them with the shims (`install_shims`), under `<runtime dir>/shell/` (0700, files 0600 or 0700 as needed), replacing whatever is there; written to a temp file and renamed so a pane starting meanwhile never reads half a file. When they cannot be written, it logs once and starts shells as today.

## Reload and reconnect

Nothing new is handed over. A running shell has read its wrapper already. The successor rewrites `shell/` at its start (same content unless it changed), so panes made after a reload use the new executable's wrappers. Client reconnects are unaffected.

## Alternatives considered

- A `PROMPT_COMMAND` in the environment for bash: lost when a user's file assigns `PROMPT_COMMAND`.
- Starting a non-login shell: changes which files run; the login shell is a recorded decision (archived `2026-10-11-login-shell-panes`).
- Rewriting the user's dotfiles: never.

## Changed while implementing

- bash in posix mode keeps its history in `~/.sh_history`; when the pane has no `HISTFILE`, it gets `HISTFILE=~/.bash_history` and `TILR_BASH_UNEXPORT_HISTFILE=1`, and the wrapper un-exports `HISTFILE` again (as kitty does), so history stays where bash keeps it.
- fish needs no file (its fix is the `--init-command` text), so it is wrapped whenever the panes have tools, also when the files under `shell/` could not be written; zsh and bash start as today then. A failed write also removes `shell/`, so a stale copy is never used.
- The wrappers are written by `set_up_shims` (supervisor.rs) only when there are tools for the panes, with `reload::write_file` (temp file, sync, rename; files 0600 in 0700 directories).
