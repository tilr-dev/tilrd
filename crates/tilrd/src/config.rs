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
//! # Serve the SOCKS proxy (see socks.rs), as --socks does.
//! socks = true
//! # SSH agent forwarding (see agent.rs); false does as --no-agent. Default on.
//! agent = true
//! # Exec: clients run programs outside a pane (see exec.rs); false does as
//! # --no-exec. Default on.
//! exec = true
//! # Release channel for tilrd update: stable (default) or beta.
//! update_channel = "stable"
//!
//! # The files protocol (see files.rs).
//! [files]
//! # Off: clients cannot see or touch files, as --no-files does. Default on.
//! enabled = true
//! # The directories clients may go into. Default: the home directory.
//! roots = ["~", "/srv/www"]
//! # Clients may look but not change anything. Default off.
//! readonly = false
//! # Largest file content one read returns or one write takes: bytes, or a
//! # size like "64MB". Default 64MB.
//! max_bytes = "64MB"
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
    /// Channel installed by tilrd update. Unset: stable.
    #[serde(default)]
    pub update_channel: crate::update::Channel,
    /// Browser origins allowed to connect. Unset: the built-in list.
    pub allow_origins: Option<Vec<String>>,
    /// Serve the SOCKS proxy. Unset: off.
    pub socks: Option<bool>,
    /// Forward the SSH agent of a client to the panes. Unset: on.
    pub agent: Option<bool>,
    /// Let clients run programs outside a pane. Unset: on.
    pub exec: Option<bool>,
    /// The files protocol.
    #[serde(default)]
    pub files: FilesConfig,
}

/// `[files]`: what clients may do with the files of this machine.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    /// Unset: on.
    pub enabled: Option<bool>,
    /// Unset: the home directory.
    pub roots: Option<Vec<String>>,
    /// Unset: off.
    pub readonly: Option<bool>,
    /// Unset: [`DEFAULT_FILES_MAX`].
    pub max_bytes: Option<Size>,
}

/// A size in bytes, written as a number or as text like "64MB".
#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Size {
    Bytes(u64),
    Text(String),
}

/// Largest file content one read returns or one write takes, unless the
/// file says.
pub const DEFAULT_FILES_MAX: u64 = 64 << 20;

/// What the files protocol is allowed, from the file and `--no-files`.
/// `None`: off.
pub fn files(no_files: bool, config: &Config) -> Result<Option<crate::files::Settings>> {
    let files = &config.files;
    if no_files || files.enabled == Some(false) {
        return Ok(None);
    }
    let max_bytes = match &files.max_bytes {
        None => DEFAULT_FILES_MAX,
        Some(Size::Bytes(bytes)) => *bytes,
        Some(Size::Text(text)) => crate::clipboard::parse_size(text)
            .map_err(|err| anyhow::anyhow!("[files] max_bytes: {err}"))?,
    };
    Ok(Some(crate::files::Settings {
        roots: files.roots.clone().unwrap_or_else(|| vec!["~".into()]),
        readonly: files.readonly.unwrap_or(false),
        max_bytes,
    }))
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

/// Whether to serve the SOCKS proxy: `--socks` turns it on, else the file
/// says, else it is off.
pub fn socks(cli: bool, config: &Config) -> bool {
    cli || config.socks.unwrap_or(false)
}

/// Whether to forward the SSH agent: `--no-agent` turns it off, else the
/// file says, else it is on.
pub fn agent(no_agent: bool, config: &Config) -> bool {
    !no_agent && config.agent.unwrap_or(true)
}

/// Whether clients may run programs (Exec): `--no-exec` turns it off, else
/// the file's `exec`, else on.
pub fn exec(no_exec: bool, config: &Config) -> bool {
    !no_exec && config.exec.unwrap_or(true)
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
    fn the_socks_proxy_is_off_unless_asked_for() {
        assert!(!socks(false, &Config::default()));
        assert!(socks(true, &Config::default()));
        let config: Config = toml::from_str("socks = true").unwrap();
        assert!(socks(false, &config));
    }

    #[test]
    fn exec_is_on_unless_turned_off() {
        assert!(exec(false, &Config::default()));
        assert!(!exec(true, &Config::default()));
        let config: Config = toml::from_str("exec = false").unwrap();
        assert!(!exec(false, &config));
    }

    #[test]
    fn the_agent_is_forwarded_unless_turned_off() {
        assert!(agent(false, &Config::default()));
        assert!(!agent(true, &Config::default()));
        let config: Config = toml::from_str("agent = false").unwrap();
        assert!(!agent(false, &config));
        let config: Config = toml::from_str("agent = true").unwrap();
        assert!(!agent(true, &config));
    }

    #[test]
    fn an_unknown_key_is_an_error() {
        assert!(toml::from_str::<Config>("allow_origin = []").is_err());
        assert!(toml::from_str::<Config>("[files]\nroot = []").is_err());
    }

    #[test]
    fn files_are_on_for_the_home_directory_unless_said_otherwise() {
        let files = files(false, &Config::default()).unwrap().unwrap();
        assert_eq!(files.roots, ["~"]);
        assert!(!files.readonly);
        assert_eq!(files.max_bytes, DEFAULT_FILES_MAX);
        assert!(super::files(true, &Config::default()).unwrap().is_none());

        let config: Config =
            toml::from_str("[files]\nroots = [\"/srv\"]\nreadonly = true\nmax_bytes = \"1MB\"")
                .unwrap();
        let files = super::files(false, &config).unwrap().unwrap();
        assert_eq!(files.roots, ["/srv"]);
        assert!(files.readonly);
        assert_eq!(files.max_bytes, 1 << 20);
        let config: Config = toml::from_str("[files]\nmax_bytes = 1000").unwrap();
        assert_eq!(
            super::files(false, &config).unwrap().unwrap().max_bytes,
            1000
        );

        let config: Config = toml::from_str("[files]\nenabled = false").unwrap();
        assert!(super::files(false, &config).unwrap().is_none());
        let config: Config = toml::from_str("[files]\nmax_bytes = \"lots\"").unwrap();
        assert!(super::files(false, &config).is_err());
    }
}
