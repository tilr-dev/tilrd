//! Startup wrappers that keep the pane tools first on `PATH`.
//!
//! A pane runs the user's login shell with the shim directory first on its
//! `PATH` (`TILR_SHIMS`), but the shell's startup files run after that and
//! may set `PATH` from scratch (Debian's /etc/profile, a `PATH=...` without
//! `$PATH` in ~/.profile). For zsh, bash and fish the pane starts the shell
//! through a wrapper of the server's that runs the user's files as that
//! login shell would and then puts `$TILR_SHIMS` first (once), as terminals
//! do for their shell integration:
//!
//! - zsh: `ZDOTDIR` points at `<runtime dir>/shell/zsh`, whose `.zshenv`
//!   restores the user's `ZDOTDIR` (kept in `TILR_ZDOTDIR`), sources the
//!   user's `.zshenv` and adds a one-shot `precmd` hook; zsh then reads the
//!   user's `.zprofile`, `.zshrc` and `.zlogin` as usual.
//! - bash: `-bash --posix` with `ENV=<runtime dir>/shell/bash/env.sh`: in
//!   posix mode bash reads only `$ENV`, which leaves posix mode, runs
//!   /etc/profile and the first of ~/.bash_profile, ~/.bash_login,
//!   ~/.profile, then fixes `PATH`.
//! - fish: `--init-command`, evaluated after fish's configuration.
//!
//! Other shells start as before; the `TILR_SHIMS` hint is for them.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Where the wrappers are, under the runtime directory.
pub fn wrapper_dir(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("shell")
}

/// The user's `ZDOTDIR`, kept while zsh reads the wrapper's `.zshenv`.
pub const ZDOTDIR_ENV: &str = "TILR_ZDOTDIR";
/// Set when the pane set `HISTFILE` only because posix mode would use
/// ~/.sh_history; the wrapper un-exports it again.
pub const HISTFILE_ENV: &str = "TILR_BASH_UNEXPORT_HISTFILE";

/// zsh's `.zshenv` in `shell/zsh`.
pub const ZSHENV: &str = r#"# Written by tilrd: the user's zsh startup files, then the pane tools ($TILR_SHIMS)
# first on PATH. Replaced at every start of the server.
if [[ -n "${TILR_ZDOTDIR+x}" ]]; then
  ZDOTDIR="$TILR_ZDOTDIR"
  builtin unset TILR_ZDOTDIR
else
  builtin unset ZDOTDIR
fi
__tilr_file="${ZDOTDIR:-$HOME}/.zshenv"
if [[ -r "$__tilr_file" ]]; then
  builtin source "$__tilr_file"
fi
builtin unset __tilr_file
__tilr_path_first() {
  if [[ -n "$TILR_SHIMS" ]]; then
    path=("$TILR_SHIMS" ${path:#${(b)TILR_SHIMS}})
  fi
  precmd_functions=(${precmd_functions:#__tilr_path_first})
  builtin unfunction __tilr_path_first
}
builtin typeset -ga precmd_functions
precmd_functions+=(__tilr_path_first)
"#;

/// bash's `$ENV` in `shell/bash`.
pub const BASH_ENV_FILE: &str = r#"# Written by tilrd: what a bash login shell reads, then the pane tools ($TILR_SHIMS)
# first on PATH. Replaced at every start of the server.
builtin unset ENV
builtin set +o posix
if [ -n "${TILR_BASH_UNEXPORT_HISTFILE-}" ]; then
  builtin export -n HISTFILE
  builtin unset TILR_BASH_UNEXPORT_HISTFILE
fi
if [ -r /etc/profile ]; then
  builtin source /etc/profile
fi
for __tilr_file in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
  if [ -r "$__tilr_file" ]; then
    builtin source "$__tilr_file"
    break
  fi
done
builtin unset __tilr_file
if [ -n "${TILR_SHIMS-}" ]; then
  __tilr_path=":$PATH:"
  while [[ "$__tilr_path" == *":$TILR_SHIMS:"* ]]; do
    __tilr_path="${__tilr_path//":$TILR_SHIMS:"/:}"
  done
  __tilr_path="${__tilr_path#:}"
  __tilr_path="${__tilr_path%:}"
  PATH="$TILR_SHIMS${__tilr_path:+:$__tilr_path}"
  builtin unset __tilr_path
fi
"#;

/// fish's `--init-command`.
pub const FISH_INIT: &str =
    "if set -q TILR_SHIMS; set -gx PATH $TILR_SHIMS (string match -v -- $TILR_SHIMS $PATH); end";

/// The wrapper files: path under the wrapper directory, content.
const FILES: [(&str, &str); 2] = [("zsh/.zshenv", ZSHENV), ("bash/env.sh", BASH_ENV_FILE)];

/// Write the wrappers under `<runtime dir>/shell/` (0700 directories, 0600
/// files), each replaced whole. Returns the directory.
pub fn install_wrappers(runtime_dir: &Path) -> Result<PathBuf> {
    let dir = wrapper_dir(runtime_dir);
    crate::clipboard::private_dir(&dir)?;
    for (name, text) in FILES {
        // Written aside, synced and renamed: a pane starting meanwhile never
        // reads half a file.
        crate::reload::write_file(&dir.join(name), text.as_bytes())?;
    }
    Ok(dir)
}

/// Whether the wrappers for `name` (`zsh`, `bash`) are in `dir`.
pub fn has_wrapper(dir: &Path, name: &str) -> bool {
    FILES
        .iter()
        .filter(|(file, _)| file.starts_with(&format!("{name}/")))
        .all(|(file, _)| dir.join(file).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_wrappers_are_written_private_and_replaced_whole() {
        let runtime = std::env::temp_dir().join(format!("tilrd-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let dir = install_wrappers(&runtime).unwrap();
        assert_eq!(dir, runtime.join("shell"));
        assert_eq!(mode(&dir), 0o700);
        for (name, text) in FILES {
            let file = dir.join(name);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
            assert_eq!(mode(&file), 0o600, "{name}");
            assert_eq!(mode(file.parent().unwrap()), 0o700, "{name}");
        }
        assert!(has_wrapper(&dir, "zsh") && has_wrapper(&dir, "bash"));

        // An older or damaged copy is replaced, and nothing is left beside it.
        std::fs::write(dir.join("zsh/.zshenv"), "old").unwrap();
        install_wrappers(&runtime).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("zsh/.zshenv")).unwrap(),
            ZSHENV
        );
        let mut names: Vec<_> = std::fs::read_dir(dir.join("zsh"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, [".zshenv"]);
        std::fs::remove_dir_all(&runtime).unwrap();
    }

    #[test]
    fn no_wrappers_means_shells_start_as_before() {
        let missing = Path::new("/nonexistent/tilr/shell");
        assert!(!has_wrapper(missing, "zsh") && !has_wrapper(missing, "bash"));
        let env = |_: &str| None;
        for shell in ["/bin/zsh", "/bin/bash"] {
            let plan = crate::pty::login_plan(shell, Some(missing), true, &env);
            assert_eq!(plan.args, Vec::<String>::new(), "{shell}");
            assert!(plan.env.is_empty(), "{shell}");
        }
        // No tools for the panes: nothing to put first, nothing wrapped.
        let plan = crate::pty::login_plan("/usr/bin/fish", None, false, &env);
        assert_eq!(plan.arg0, "-fish");
        assert!(plan.args.is_empty() && plan.env.is_empty());
    }
}
