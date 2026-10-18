## ADDED Requirements

### Requirement: The tools stay first on PATH after the login profile
When a pane runs the user's login shell, that shell is zsh, bash or fish, and the server has tools for the panes, the first command at the shell's first prompt SHALL see `$TILR_SHIMS` as the first directory of `PATH` and nowhere else on it, even when the startup files set `PATH` from scratch; the shell SHALL still read the user's startup files a login shell of its kind reads and SHALL still run as `-<name>`.

**Verified by**: itests `UC-07B` (`uc_07b.py::the_tools_stay_first_on_path_when_the_profile_resets_it`, `uc_07b.py::the_users_startup_files_still_run`).

#### Scenario: bash with a profile that resets PATH
- **WHEN** the login shell is bash and `~/.bash_profile` says `PATH=/usr/bin:/bin`
- **THEN** `echo "$PATH"` in a new pane prints `$TILR_SHIMS:/usr/bin:/bin`

#### Scenario: zsh and fish
- **WHEN** the login shell is zsh with `path=(/usr/bin /bin)` in `~/.zprofile`, or fish with `set -gx PATH /usr/bin /bin` in `config.fish`
- **THEN** `$TILR_SHIMS` is the first directory of `PATH` in a new pane

#### Scenario: The user's files still run
- **WHEN** `~/.zshrc`, `~/.bash_profile` or `config.fish` sets a variable `X=1`, and zsh's `ZDOTDIR` is the user's own
- **THEN** `echo $X` prints `1`, `$0` is `-zsh`, `-bash` or `-fish`, and in zsh `$ZDOTDIR` is the user's value or unset as it was

#### Scenario: Another shell
- **WHEN** the login shell is `/bin/sh` (dash) and `~/.profile` resets `PATH`
- **THEN** the pane starts as before and `$TILR_SHIMS` is still set for the user's own `PATH="$TILR_SHIMS:$PATH"`
