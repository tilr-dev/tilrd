//! Which server `tilr` talks to: the local tilrd on its Unix socket (the
//! default; started if it is not running), or a tilrd's TCP port.
//!
//! The command line wins, then the environment (`TILR_SOCKET`,
//! `TILR_CONNECT`, `TILR_TOKEN`), then the configuration file, then the
//! default socket (see [`tilr_proto::local`]). The file is
//! `$TILR_CONFIG_DIR/client.toml`, else `$XDG_CONFIG_HOME/tilr/client.toml`
//! (`~/.config/tilr/client.toml`), next to the server's `server.toml`:
//!
//! ```toml
//! # A tilrd elsewhere instead of the local one:
//! connect = "ws://10.0.0.5:7890"
//! token = "…"            # what `tilrd token` prints there
//! # Or the local one at another path:
//! # socket = "/run/user/1000/tilr/tilrd.sock"
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const CONFIG_DIR_ENV: &str = "TILR_CONFIG_DIR";
const FILE: &str = "client.toml";

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// The tilrd of this user on this machine, at this socket.
    Local(PathBuf),
    /// A tilrd's WebSocket port, with its access token.
    Remote { url: String, token: String },
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local(path) => write!(f, "{}", path.display()),
            Self::Remote { url, .. } => write!(f, "{url}"),
        }
    }
}

/// What the configuration file may say.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub socket: Option<PathBuf>,
    pub connect: Option<String>,
    pub token: Option<String>,
}

/// What the command line (and the environment, through clap) said.
#[derive(Debug, Default)]
pub struct Given {
    pub socket: Option<PathBuf>,
    pub connect: Option<String>,
    pub token: Option<String>,
}

pub fn config_path() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|dir| !dir.is_empty());
    if let Some(dir) = set(CONFIG_DIR_ENV) {
        return Some(PathBuf::from(dir).join(FILE));
    }
    if let Some(dir) = set("XDG_CONFIG_HOME") {
        return Some(Path::new(&dir).join("tilr").join(FILE));
    }
    Some(std::env::home_dir()?.join(".config/tilr").join(FILE))
}

/// The configuration file; a missing one says nothing.
pub fn load_config() -> Result<Config> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("{}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

/// Settle on the server: a socket or an address given beats one from the
/// file; neither, the default socket.
pub fn resolve(given: Given, config: Config) -> Result<Target> {
    if given.socket.is_some() && given.connect.is_some() {
        bail!("--socket and --connect name two servers: give one");
    }
    if let Some(path) = given.socket {
        return Ok(Target::Local(path));
    }
    let url = given.connect.or(if config.socket.is_none() {
        config.connect
    } else {
        None
    });
    match url {
        Some(url) => {
            let token = given
                .token
                .or(config.token)
                .or_else(stored_token)
                .unwrap_or_default();
            Ok(Target::Remote { url, token })
        }
        None => Ok(Target::Local(
            config
                .socket
                .unwrap_or_else(tilr_proto::local::default_socket_path),
        )),
    }
}

/// The token of the server on this machine (what `tilrd token` prints), for
/// `--connect ws://127.0.0.1:…` without `--token`.
fn stored_token() -> Option<String> {
    let set = |name: &str| std::env::var_os(name).filter(|dir| !dir.is_empty());
    let dir = set("TILR_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(set("XDG_STATE_HOME")?).join("tilr")))
        .or_else(|| Some(std::env::home_dir()?.join(".local/state/tilr")))?;
    let token = std::fs::read_to_string(dir.join("token")).ok()?;
    Some(token.trim().to_owned()).filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn given(socket: Option<&str>, connect: Option<&str>, token: Option<&str>) -> Given {
        Given {
            socket: socket.map(PathBuf::from),
            connect: connect.map(str::to_owned),
            token: token.map(str::to_owned),
        }
    }

    #[test]
    fn the_command_line_beats_the_file() {
        let config: Config = toml::from_str("connect = \"ws://far:7890\"\ntoken = \"t1\"").unwrap();
        assert_eq!(
            resolve(given(Some("/s"), None, None), config).unwrap(),
            Target::Local("/s".into())
        );
        let config: Config = toml::from_str("socket = \"/from-file\"").unwrap();
        assert_eq!(
            resolve(given(None, Some("ws://h:1"), Some("t")), config).unwrap(),
            Target::Remote {
                url: "ws://h:1".into(),
                token: "t".into()
            }
        );
    }

    #[test]
    fn the_file_names_a_server_and_its_token() {
        let config: Config = toml::from_str("connect = \"ws://far:7890\"\ntoken = \"t1\"").unwrap();
        assert_eq!(
            resolve(Given::default(), config).unwrap(),
            Target::Remote {
                url: "ws://far:7890".into(),
                token: "t1".into()
            }
        );
        let config: Config = toml::from_str("socket = \"/s\"").unwrap();
        assert_eq!(
            resolve(Given::default(), config).unwrap(),
            Target::Local("/s".into())
        );
    }

    #[test]
    fn both_a_socket_and_an_address_is_an_error() {
        let err = resolve(given(Some("/s"), Some("ws://h:1"), None), Config::default());
        assert!(err.is_err());
        assert!(toml::from_str::<Config>("sockett = \"/s\"").is_err());
    }
}
