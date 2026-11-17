//! `bus.toml` (B10, M4): modules, instances and their rights, read at start and on
//! `bus.reload_config`.
//!
//! Owner: `bus-instances`.
//!
//! ```toml
//! call_timeout_ms = 10000      # default 10000
//! drain_ms = 3000              # default 3000, at most 30000
//! [modules.deno]
//! command = ["deno", "run", "-A", "main.ts"]   # required, non-empty
//! cwd = "~/tilr/components"    # absolute or ~; default the configuration directory
//! [modules.deno.env]
//! LOG = "info"
//! [instances.deno-1]
//! module = "deno"              # required
//! connect = "fd"               # "fd" (default) | "socket"
//! autostart = true             # default true
//! [instances.deno-1.rights]
//! call = ["math.*"]
//! provide = ["ui.*"]
//! [instances.deno-1.env]
//! WORKERS = "2"
//! ```
//!
//! Strict: an unknown key, a wrong type, an instance naming no module, an empty
//! command, a reserved or unusable instance name and a socket path over the platform
//! limit are errors, reported with the file and the reason. A missing file is an empty
//! configuration. [`diff`] says what a changed file means for the running instances.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::identity::Rights;

/// The file's name in the configuration directory.
pub const FILE: &str = "bus.toml";

/// `call_timeout_ms` when the file does not say.
pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 10_000;
/// `drain_ms` when the file does not say.
pub const DEFAULT_DRAIN_MS: u64 = 3_000;
/// The longest drain, in `bus.toml` and in `bus.stop`: a stop in progress must not keep
/// a reload or a shutdown of the server waiting without end.
pub const MAX_DRAIN_MS: u64 = 30_000;

/// The longest `sun_path` of a Unix socket, the terminating NUL included.
#[cfg(target_os = "macos")]
pub const SOCKET_PATH_LIMIT: usize = 104;
/// The longest `sun_path` of a Unix socket, the terminating NUL included.
#[cfg(not(target_os = "macos"))]
pub const SOCKET_PATH_LIMIT: usize = 108;

/// What `bus.toml` says.
#[derive(Clone, Debug, PartialEq)]
pub struct BusConfig {
    pub call_timeout_ms: u64,
    pub drain_ms: u64,
    pub modules: BTreeMap<String, Module>,
    pub instances: BTreeMap<String, InstanceSpec>,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            call_timeout_ms: DEFAULT_CALL_TIMEOUT_MS,
            drain_ms: DEFAULT_DRAIN_MS,
            modules: BTreeMap::new(),
            instances: BTreeMap::new(),
        }
    }
}

/// `[modules.<name>]`: what to run.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub command: Vec<String>,
    /// Absolute or `~`; unset: the configuration directory.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// How an instance's bus connection reaches the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Connect {
    /// Descriptor 3, `TILR_BUS_FD`.
    #[default]
    Fd,
    /// An instance socket, `TILR_BUS_SOCK` and `TILR_BUS_TOKEN`.
    Socket,
}

impl Connect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fd => "fd",
            Self::Socket => "socket",
        }
    }
}

/// `[instances.<name>]`: one process of a module and what it may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceSpec {
    pub module: String,
    pub connect: Connect,
    pub autostart: bool,
    pub rights: Rights,
    pub env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    call_timeout_ms: Option<u64>,
    drain_ms: Option<u64>,
    #[serde(default)]
    modules: BTreeMap<String, Module>,
    #[serde(default)]
    instances: BTreeMap<String, RawInstance>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstance {
    module: String,
    #[serde(default)]
    connect: Connect,
    autostart: Option<bool>,
    #[serde(default)]
    rights: Rights,
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// The first round's key: only here to name its replacement.
    calls: Option<toml::Value>,
}

/// Why `bus.toml` was refused: names the file and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Where the configuration directory is: the one `server.toml` is looked for in.
pub fn config_dir() -> Option<PathBuf> {
    crate::config::default_path()?
        .parent()
        .map(Path::to_path_buf)
}

/// Read `bus.toml` of `dir`; a missing file is an empty configuration. `runtime_dir`
/// is where instance sockets go (their path length is checked here).
pub fn load(dir: &Path, runtime_dir: &Path) -> Result<BusConfig, ConfigError> {
    let path = dir.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BusConfig::default());
        }
        Err(err) => return Err(ConfigError(format!("{}: {err}", path.display()))),
    };
    parse(&text, runtime_dir)
        .map_err(|ConfigError(reason)| ConfigError(format!("{}: {reason}", path.display())))
}

/// Parse and check the text of `bus.toml`.
pub fn parse(text: &str, runtime_dir: &Path) -> Result<BusConfig, ConfigError> {
    let raw: RawConfig = toml::from_str(text).map_err(|err| {
        let line = err
            .span()
            .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
        match line {
            Some(line) => ConfigError(format!("line {line}: {}", err.message())),
            None => ConfigError(err.message().to_owned()),
        }
    })?;
    let mut config = BusConfig {
        call_timeout_ms: raw.call_timeout_ms.unwrap_or(DEFAULT_CALL_TIMEOUT_MS),
        drain_ms: raw.drain_ms.unwrap_or(DEFAULT_DRAIN_MS),
        modules: raw.modules,
        instances: BTreeMap::new(),
    };
    if config.drain_ms > MAX_DRAIN_MS {
        return Err(ConfigError(format!(
            "drain_ms = {} is over the limit of {MAX_DRAIN_MS}",
            config.drain_ms
        )));
    }
    for (name, module) in &config.modules {
        if name.is_empty() {
            return Err(ConfigError("a module has an empty name".into()));
        }
        if module.command.first().is_none_or(String::is_empty) {
            return Err(ConfigError(format!(
                "[modules.{name}] command is empty: name the program to run"
            )));
        }
    }
    for (name, instance) in raw.instances {
        check_name(&name)?;
        if instance.calls.is_some() {
            return Err(ConfigError(format!(
                "[instances.{name}] `calls` was replaced by `[instances.{name}.rights] call`"
            )));
        }
        if !config.modules.contains_key(&instance.module) {
            return Err(ConfigError(format!(
                "[instances.{name}] module {:?} is not defined in [modules]",
                instance.module
            )));
        }
        if instance.connect == Connect::Socket {
            let path = socket_path(runtime_dir, &name);
            let length = path.as_os_str().len();
            if length >= SOCKET_PATH_LIMIT {
                return Err(ConfigError(format!(
                    "[instances.{name}] the socket path {} is {length} bytes, over the limit of {} for a Unix socket here",
                    path.display(),
                    SOCKET_PATH_LIMIT - 1
                )));
            }
        }
        config.instances.insert(
            name,
            InstanceSpec {
                module: instance.module,
                connect: instance.connect,
                autostart: instance.autostart.unwrap_or(true),
                rights: instance.rights,
                env: instance.env,
            },
        );
    }
    Ok(config)
}

/// An instance's name is its identity (`instance:<name>`) and the name of its socket
/// file: letters, digits, `.`, `_` and `-`, not starting with `.` or `-`; `owner`,
/// `tilrd` and names starting with `bus` are reserved (B10).
fn check_name(name: &str) -> Result<(), ConfigError> {
    let bad = |why: &str| Err(ConfigError(format!("[instances.{name}] the name {why}")));
    if name.is_empty() {
        return bad("is empty");
    }
    if name == "owner" || name == "tilrd" || name.starts_with("bus") {
        return bad("is reserved (`owner`, `tilrd` and names starting with `bus`)");
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap_or('.');
    if !first.is_ascii_alphanumeric() {
        return bad("must start with a letter or a digit");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return bad("may hold only letters, digits, `.`, `_` and `-`");
    }
    Ok(())
}

/// `<runtime dir>/bus/<name>.sock`.
pub fn socket_path(runtime_dir: &Path, name: &str) -> PathBuf {
    runtime_dir.join("bus").join(format!("{name}.sock"))
}

/// The directory a module starts in: its `cwd` (`~` is the home directory; a
/// relative path is under the configuration directory), else the configuration
/// directory.
pub fn working_dir(module: &Module, config_dir: &Path) -> PathBuf {
    let Some(cwd) = module.cwd.as_deref().filter(|cwd| !cwd.is_empty()) else {
        return config_dir.to_path_buf();
    };
    let home = || std::env::home_dir().unwrap_or_else(|| config_dir.to_path_buf());
    if cwd == "~" {
        home()
    } else if let Some(rest) = cwd.strip_prefix("~/") {
        home().join(rest)
    } else {
        config_dir.join(cwd)
    }
}

/// What a new configuration means for the instances of the old one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// In the new, not in the old.
    pub added: Vec<String>,
    /// In the old, not in the new.
    pub removed: Vec<String>,
    /// In both, with another module definition or another entry (`autostart` aside:
    /// it matters at start only).
    pub changed: Vec<String>,
    /// In both, the same.
    pub kept: Vec<String>,
}

/// Compare two configurations, instance by instance, in name order.
pub fn diff(old: &BusConfig, new: &BusConfig) -> Diff {
    let mut diff = Diff::default();
    for (name, spec) in &new.instances {
        let Some(before) = old.instances.get(name) else {
            diff.added.push(name.clone());
            continue;
        };
        let same_entry = InstanceSpec {
            autostart: spec.autostart,
            ..before.clone()
        } == *spec;
        if same_entry && old.modules.get(&before.module) == new.modules.get(&spec.module) {
            diff.kept.push(name.clone());
        } else {
            diff.changed.push(name.clone());
        }
    }
    for name in old.instances.keys() {
        if !new.instances.contains_key(name) {
            diff.removed.push(name.clone());
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> PathBuf {
        PathBuf::from("/run/tilr")
    }

    fn refused(text: &str) -> String {
        parse(text, &runtime()).unwrap_err().0
    }

    const MODULE: &str = "[modules.t]\ncommand = [\"/bin/true\"]\n";

    #[test]
    fn bus_toml_is_read_with_its_defaults() {
        let config = parse(
            &format!(
                "{MODULE}[instances.t-1]\nmodule = \"t\"\n[instances.t-1.rights]\nprovide = [\"test.*\"]\n[instances.t-1.env]\nA = \"1\"\n"
            ),
            &runtime(),
        )
        .unwrap();
        assert_eq!(config.call_timeout_ms, 10_000);
        assert_eq!(config.drain_ms, 3_000);
        let instance = &config.instances["t-1"];
        assert_eq!(instance.connect, Connect::Fd);
        assert!(instance.autostart);
        assert_eq!(instance.rights.provide, ["test.*"]);
        assert_eq!(instance.env["A"], "1");
        assert!(parse("", &runtime()).unwrap().instances.is_empty());
    }

    #[test]
    fn a_misspelt_key_is_named() {
        let reason = refused(&format!("{MODULE}[instances.t-1]\nmodul = \"t\"\n"));
        assert!(reason.contains("modul"), "{reason}");
        assert!(reason.contains("line "), "{reason}");
        let reason = refused("call_timeout = 5\n");
        assert!(reason.contains("call_timeout"), "{reason}");
        let reason = refused(&format!(
            "{MODULE}[instances.t-1]\nmodule = \"t\"\n[instances.t-1.rights]\ncals = []\n"
        ));
        assert!(reason.contains("cals"), "{reason}");
    }

    #[test]
    fn the_first_rounds_calls_key_names_rights() {
        let reason = refused(&format!(
            "{MODULE}[instances.t-1]\nmodule = \"t\"\ncalls = [\"test.*\"]\n"
        ));
        assert!(
            reason.contains("`calls` was replaced by `[instances.t-1.rights] call`"),
            "{reason}"
        );
    }

    #[test]
    fn a_drain_over_the_limit_is_refused() {
        let reason = refused("drain_ms = 30001\n");
        assert!(
            reason.contains("drain_ms = 30001 is over the limit of 30000"),
            "{reason}"
        );
        assert_eq!(
            parse("drain_ms = 30000\n", &runtime()).unwrap().drain_ms,
            30_000
        );
    }

    #[test]
    fn a_wrong_type_is_refused() {
        for text in [
            "drain_ms = \"soon\"\n".to_owned(),
            format!("{MODULE}[instances.t-1]\nmodule = \"t\"\nautostart = \"yes\"\n"),
            format!("{MODULE}[instances.t-1]\nmodule = \"t\"\nconnect = \"pipe\"\n"),
            "[modules.t]\ncommand = \"/bin/true\"\n".to_owned(),
            format!("{MODULE}[instances.t-1]\nmodule = \"t\"\n[instances.t-1.env]\nA = 1\n"),
        ] {
            assert!(parse(&text, &runtime()).is_err(), "{text}");
        }
    }

    #[test]
    fn an_instance_needs_a_module_and_a_module_a_command() {
        let reason = refused("[instances.t-1]\nmodule = \"t\"\n");
        assert!(reason.contains("module \"t\""), "{reason}");
        let reason = refused("[modules.t]\ncommand = []\n");
        assert!(reason.contains("command is empty"), "{reason}");
        let reason = refused("[modules.t]\ncommand = [\"\"]\n");
        assert!(reason.contains("command is empty"), "{reason}");
        let reason = refused("[modules.t]\n");
        assert!(reason.contains("command"), "{reason}");
    }

    #[test]
    fn reserved_and_unusable_instance_names_are_refused() {
        for name in [
            "owner", "tilrd", "bus", "bus-1", "busy", "a/b", "-x", ".x", "a b",
        ] {
            let reason = refused(&format!("{MODULE}[instances.\"{name}\"]\nmodule = \"t\"\n"));
            assert!(reason.contains(name), "{name}: {reason}");
        }
        assert!(
            parse(
                &format!("{MODULE}[instances.t-1.x_y]\nmodule = \"t\"\n"),
                &runtime()
            )
            .is_err()
        );
    }

    #[test]
    fn a_socket_path_over_the_platform_limit_is_refused() {
        let name = "x".repeat(SOCKET_PATH_LIMIT);
        let text = format!("{MODULE}[instances.{name}]\nmodule = \"t\"\nconnect = \"socket\"\n");
        let reason = refused(&text);
        assert!(reason.contains("socket path"), "{reason}");
        // The same name is fine for descriptor 3.
        assert!(parse(&text.replace("socket", "fd"), &runtime()).is_ok());
        // And a short one is fine on a socket.
        assert!(
            parse(
                &format!("{MODULE}[instances.t-1]\nmodule = \"t\"\nconnect = \"socket\"\n"),
                &runtime()
            )
            .is_ok()
        );
    }

    #[test]
    fn a_changed_file_is_compared_instance_by_instance() {
        let old = parse(
            &format!(
                "{MODULE}[instances.a]\nmodule = \"t\"\n[instances.b]\nmodule = \"t\"\n[instances.c]\nmodule = \"t\"\n[instances.d]\nmodule = \"t\"\n"
            ),
            &runtime(),
        )
        .unwrap();
        let new = parse(
            &format!(
                "{MODULE}[instances.a]\nmodule = \"t\"\n[instances.a.env]\nX = \"1\"\n[instances.b]\nmodule = \"t\"\nautostart = false\n[instances.d]\nmodule = \"t\"\n[instances.e]\nmodule = \"t\"\n"
            ),
            &runtime(),
        )
        .unwrap();
        let diff = diff(&old, &new);
        assert_eq!(diff.added, ["e"]);
        assert_eq!(diff.removed, ["c"]);
        assert_eq!(diff.changed, ["a"]);
        // `autostart` matters at start only.
        assert_eq!(diff.kept, ["b", "d"]);
        // A changed module changes its instances.
        let moved = parse(
            "[modules.t]\ncommand = [\"/bin/false\"]\n[instances.a]\nmodule = \"t\"\n",
            &runtime(),
        )
        .unwrap();
        assert_eq!(super::diff(&old, &moved).changed, ["a"]);
    }

    #[test]
    fn the_working_directory_defaults_to_the_configuration_directory() {
        let config_dir = Path::new("/etc/tilr");
        let module = |cwd: Option<&str>| Module {
            command: vec!["x".into()],
            cwd: cwd.map(str::to_owned),
            env: BTreeMap::new(),
        };
        assert_eq!(working_dir(&module(None), config_dir), config_dir);
        assert_eq!(
            working_dir(&module(Some("/srv/x")), config_dir),
            Path::new("/srv/x")
        );
        assert_eq!(
            working_dir(&module(Some("sub")), config_dir),
            Path::new("/etc/tilr/sub")
        );
        if let Some(home) = std::env::home_dir() {
            assert_eq!(
                working_dir(&module(Some("~/c")), config_dir),
                home.join("c")
            );
        }
    }
}
