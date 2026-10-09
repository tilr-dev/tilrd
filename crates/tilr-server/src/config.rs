//! The server's configuration file: what a machine's operator sets once,
//! rather than on every command line. `$XDG_CONFIG_HOME/tilr/server.toml`
//! (`~/.config/tilr/server.toml`), or the file `--config` names. A missing
//! file is an empty configuration; a file that does not parse is an error
//! at startup, not a silent default.
//!
//! ```toml
//! # Browser origins allowed to connect (clients that send no Origin header
//! are always allowed). Replaces the built-in list, as --allow-origin does.
//! allow_origins = ["tilr://app", "tilr://tilr", "tilr://my-notes"]
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Overrides the directory the configuration file is looked for in.
const CONFIG_DIR_ENV: &str = "TILR_CONFIG_DIR";
const FILE: &str = "server.toml";

/// Browser origins allowed when neither the file nor the command line says:
/// the Tilr app's page (`tilr://app`), its terminal app (`tilr://tilr`), and
/// the web client's dev origins.
pub const DEFAULT_ORIGINS: &[&str] = &[
    "tilr://app",
    "tilr://tilr",
    "http://127.0.0.1:7891",
    "http://localhost:7891",
];

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Browser origins allowed to connect. Unset: the built-in list.
    pub allow_origins: Option<Vec<String>>,
}

/// Where the file is looked for when `--config` is not given.
pub fn default_path() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|dir| !dir.is_empty());
    if let Some(dir) = set(CONFIG_DIR_ENV) {
        return Some(PathBuf::from(dir).join(FILE));
    }
    if let Some(dir) = set("XDG_CONFIG_HOME") {
        return Some(Path::new(&dir).join("tilr").join(FILE));
    }
    Some(std::env::home_dir()?.join(".config/tilr").join(FILE))
}

/// Reads the file at `path`, or the default one. A file that is not there
/// gives the default configuration; one named by `--config` must exist.
pub fn load(path: Option<&Path>) -> Result<Config> {
    let (path, required) = match path {
        Some(path) => (path.to_path_buf(), true),
        None => match default_path() {
            Some(path) => (path, false),
            None => return Ok(Config::default()),
        },
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound && !required => {
            return Ok(Config::default());
        }
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let config: Config =
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    tracing::info!(path = %path.display(), "configuration read");
    Ok(config)
}

/// The origins to allow: the command line's when given, else the file's,
/// else the built-in list.
pub fn allow_origins(cli: Vec<String>, config: &Config) -> Vec<String> {
    if !cli.is_empty() {
        return cli;
    }
    match &config.allow_origins {
        Some(origins) => origins.clone(),
        None => DEFAULT_ORIGINS.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_sets_the_origins_and_the_command_line_wins() {
        let config: Config = toml::from_str("allow_origins = [\"tilr://my-notes\"]").unwrap();
        assert_eq!(allow_origins(vec![], &config), vec!["tilr://my-notes"]);
        assert_eq!(
            allow_origins(vec!["http://x".into()], &config),
            vec!["http://x"]
        );
        assert_eq!(allow_origins(vec![], &Config::default()), DEFAULT_ORIGINS);
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(toml::from_str::<Config>("allow_origin = []").is_err());
    }
}
