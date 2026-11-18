//! Launch profiles (shared design Part P): named files `<name>.toml` in `profiles_dir`
//! that say what a pane runs, where, with which environment and, in a `[sandbox]`
//! table, what it may touch.
//!
//! This file knows nothing of the bus or the supervisor: it finds, checks and parses a
//! profile, and resolves it into a [`Snapshot`], what the pane keeps for life (and across
//! a reload, `bus-reload`). `bus/core/profiles.rs` serves `pane.profiles` and the
//! profile half of `pane.create` with it.
//!
//! ```toml
//! description = "A shell for untrusted scripts"   # optional
//! command = ["/bin/sh"]                           # absent or empty: the login shell
//! cwd = "~/work"                                  # absolute or `~`
//! [env]
//! GREETING = "hi"                                 # not TILR_*
//! [sandbox]                                       # optional; refused where it cannot be enforced
//! read_only = ["~/docs"]
//! read_write = ["~/work"]
//! executables = ["/usr/bin/git"]
//! env = ["LANG"]
//! network = "none"                                # none (default) | client | server
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The most profiles `pane.profiles` lists.
pub const MAX_LISTED: usize = 1000;
/// Longest `description` or `error` text in a listing, in bytes.
pub const MAX_TEXT: usize = 1024;
/// Largest profile file, in bytes; a larger one does not load.
pub const MAX_FILE: u64 = 64 * 1024;

/// What a profile's `[sandbox]` table allows the network (S5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// No TCP connect or bind (default).
    #[default]
    None,
    /// Outgoing connections only.
    Client,
    /// Incoming and outgoing.
    Server,
}

/// The resolved `[sandbox]` table: absolute paths (`~` expanded, `.` and `..` folded).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSpec {
    pub read_only: Vec<PathBuf>,
    pub read_write: Vec<PathBuf>,
    pub executables: Vec<PathBuf>,
    /// Names of the server's environment variables the pane gets.
    pub env: Vec<String>,
    pub network: Network,
}

/// A profile resolved at the moment a pane is created (P4): what the pane keeps for
/// life. Nothing in it is read again from the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The profile's name (`PaneInfo.profile`).
    pub name: String,
    /// The profile's `command`; empty: the login shell. The request's command, when
    /// given, replaces it where the pane is started, not here.
    pub argv: Vec<String>,
    /// The profile's `cwd`, absolute; `None`: not set.
    pub cwd: Option<PathBuf>,
    /// The profile's `[env]`, names checked, values with a leading `~/` expanded.
    pub env: BTreeMap<String, String>,
    /// The `[sandbox]` table; `Some` makes the pane sandboxed.
    pub sandbox: Option<SandboxSpec>,
}

/// Why a profile does not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The name does not match the name rule.
    BadName(String),
    /// A valid name without a file (or without a directory).
    NoSuch(String),
    /// The file is there and unusable; the text names the file and the reason.
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadName(name) => write!(f, "bad profile name: {name}"),
            Error::NoSuch(name) => write!(f, "no such profile: {name}"),
            Error::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

/// One row of a listing (`ProfileInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub description: String,
    pub sandboxed: bool,
    /// Empty when the profile loads.
    pub error: String,
}

/// The directory the server reads profiles from, set once at start (`main`); a reload
/// execs a new process, which sets it again from its own configuration.
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Remember the profiles directory of this server.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The profiles directory of this server; `None` before [`set_dir`] (no profiles).
pub fn dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

/// A profile name (P2): `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`.
pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

// ---------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    #[serde(default)]
    description: String,
    #[serde(default)]
    command: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    sandbox: Option<RawSandbox>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSandbox {
    #[serde(default)]
    read_only: Vec<String>,
    #[serde(default)]
    read_write: Vec<String>,
    #[serde(default)]
    executables: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    network: Network,
}

/// A parsed, checked profile.
struct Parsed {
    description: String,
    snapshot: Snapshot,
}

/// The user's home directory from the password database, else `$HOME`.
pub fn home() -> Option<PathBuf> {
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let mut buf = vec![0u8; 4096];
    loop {
        // SAFETY: an all-zero passwd is a valid out-value.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: the out-pointers are valid and `buf` outlives the strings `pwd`
        // points into, which are copied before it goes.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut found,
            )
        };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc == 0 && !found.is_null() && !pwd.pw_dir.is_null() {
            // SAFETY: a non-null passwd field is a NUL-terminated string in `buf`.
            let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }.to_string_lossy();
            if !dir.is_empty() {
                return Some(PathBuf::from(dir.into_owned()));
            }
        }
        return std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from);
    }
}

/// An absolute path from `text`: absolute, or `~` / `~/…` (the home directory); `.`
/// and `..` folded lexically (`..` at the root stays at the root). `profiles_dir` is
/// resolved with it too.
pub fn resolve_path(text: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    if text.contains('\0') || text.contains('\n') {
        return Err("the path has a NUL or a newline".into());
    }
    let joined = if text == "~" || text.starts_with("~/") {
        let home = home.ok_or("the home directory is unknown")?;
        home.join(text.strip_prefix("~/").unwrap_or(""))
    } else if text.starts_with('/') {
        PathBuf::from(text)
    } else {
        return Err("the path must be absolute or start with ~".into());
    };
    let mut folded = PathBuf::from("/");
    for component in joined.components() {
        match component {
            Component::Normal(part) => folded.push(part),
            Component::ParentDir => {
                folded.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    Ok(folded)
}

fn paths(key: &str, values: &[String], home: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    values
        .iter()
        .map(|value| resolve_path(value, home).map_err(|err| format!("{key} {value:?}: {err}")))
        .collect()
}

fn check_env_name(table: &str, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("{table}: a variable name is empty"));
    }
    if name.contains('=') || name.contains('\0') {
        return Err(format!(
            "{table}: the variable name {name:?} has = or a NUL"
        ));
    }
    if name.starts_with("TILR_") {
        return Err(format!(
            "{table}: {name} cannot be set, TILR_ names are the server's"
        ));
    }
    Ok(())
}

/// Parse and check the text of a profile named `name`. `home` expands `~`.
fn parse(name: &str, text: &str, home: Option<&Path>) -> Result<Parsed, String> {
    let raw: RawProfile = toml::from_str(text).map_err(|err| err.to_string())?;
    if raw.command.iter().any(|arg| arg.contains('\0')) {
        return Err("command: an argument has a NUL".into());
    }
    let cwd = raw
        .cwd
        .as_deref()
        .map(|cwd| resolve_path(cwd, home).map_err(|err| format!("cwd {cwd:?}: {err}")))
        .transpose()?;
    let mut env = BTreeMap::new();
    for (key, value) in raw.env {
        check_env_name("[env]", &key)?;
        if value.contains('\0') {
            return Err(format!("[env]: the value of {key} has a NUL"));
        }
        let value = match (value.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
            _ => value,
        };
        env.insert(key, value);
    }
    let sandbox = raw
        .sandbox
        .map(|sandbox| -> Result<SandboxSpec, String> {
            for name in &sandbox.env {
                check_env_name("[sandbox] env", name)?;
            }
            Ok(SandboxSpec {
                read_only: paths("[sandbox] read_only", &sandbox.read_only, home)?,
                read_write: paths("[sandbox] read_write", &sandbox.read_write, home)?,
                executables: paths("[sandbox] executables", &sandbox.executables, home)?,
                env: sandbox.env,
                network: sandbox.network,
            })
        })
        .transpose()?;
    Ok(Parsed {
        description: raw.description,
        snapshot: Snapshot {
            name: name.to_owned(),
            argv: raw.command,
            cwd,
            env,
            sandbox,
        },
    })
}

/// StrictModes for one inode (P2), as sshd's `auth_secure_path` judges a file and each
/// directory above it: owned by the server's user (`euid`) or root, and not writable by
/// group or others. Pure, so a test can give it an owner it cannot make.
fn secure(path: &Path, owner: u32, mode: u32, euid: u32) -> Result<(), String> {
    if owner != euid && owner != 0 {
        return Err(format!("{} is owned by uid {owner}", path.display()));
    }
    if mode & 0o022 != 0 {
        return Err(format!("{} is writable by others", path.display()));
    }
    Ok(())
}

/// [`secure`] for `dir` and every directory above it up to `/`, stopping after the home
/// directory `home` (canonical) as sshd does. `dir` is canonical: no symlink in it.
/// A sticky world-writable directory such as `/tmp` is refused, as in sshd.
fn secure_dirs(dir: &Path, home: Option<&Path>, euid: u32) -> Result<(), String> {
    let mut next = Some(dir);
    while let Some(dir) = next {
        let meta = std::fs::metadata(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        secure(dir, meta.uid(), meta.mode(), euid)?;
        if home == Some(dir) {
            break;
        }
        next = dir.parent();
    }
    Ok(())
}

/// Open, check and read the file of profile `name` in `dir`, as sshd reads
/// `authorized_keys` (`auth_openfile`, `auth_secure_path`): the file is opened once
/// without blocking (a FIFO cannot hold the server) and judged by `fstat` of that
/// descriptor: a regular file, [`secure`]; then every directory of its real path and
/// of the real profiles directory up to `/` or the home directory; then at most
/// [`MAX_FILE`] bytes are read from the same descriptor. Returns the file's path in
/// `dir` with its text. A missing directory or file is no such profile.
fn read_text(dir: &Path, name: &str, home: Option<&Path>) -> Result<(PathBuf, String), Error> {
    if !valid_name(name) {
        return Err(Error::BadName(name.to_owned()));
    }
    let file = dir.join(format!("{name}.toml"));
    let at = |err: std::io::Error| Error::Invalid(format!("{}: {err}", file.display()));
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(&file);
    let opened = match opened {
        Ok(opened) => opened,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NoSuch(name.to_owned()));
        }
        Err(err) => return Err(at(err)),
    };
    let meta = opened.metadata().map_err(at)?;
    if !meta.is_file() {
        return Err(Error::Invalid(format!(
            "{} is not a regular file",
            file.display()
        )));
    }
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { libc::geteuid() };
    secure(&file, meta.uid(), meta.mode(), euid).map_err(Error::Invalid)?;
    let real = std::fs::canonicalize(&file).map_err(at)?;
    // The path names the inode that was opened (else it changed in between).
    let named = std::fs::metadata(&real).map_err(at)?;
    if (named.dev(), named.ino()) != (meta.dev(), meta.ino()) {
        return Err(Error::Invalid(format!(
            "{} changed while it was read",
            file.display()
        )));
    }
    let home = home.and_then(|home| std::fs::canonicalize(home).ok());
    let real_dir = std::fs::canonicalize(dir)
        .map_err(|err| Error::Invalid(format!("{}: {err}", dir.display())))?;
    let file_dir = real.parent().unwrap_or(Path::new("/"));
    secure_dirs(file_dir, home.as_deref(), euid).map_err(Error::Invalid)?;
    if real_dir != file_dir {
        secure_dirs(&real_dir, home.as_deref(), euid).map_err(Error::Invalid)?;
    }
    let mut bytes = Vec::new();
    (&opened)
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(at)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::Invalid(format!(
            "{} is larger than {MAX_FILE} bytes",
            file.display()
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Invalid(format!("{} is not UTF-8", file.display())))?;
    Ok((file, text))
}

/// Read, check and parse the file of profile `name` in `dir`: the first half of what
/// both `pane.create` and `pane.profiles` do (P2-P4). Returns the description with the
/// snapshot.
fn read(dir: &Path, name: &str, home: Option<&Path>) -> Result<Parsed, Error> {
    let (file, text) = read_text(dir, name, home)?;
    parse(name, &text, home)
        .map_err(|reason| Error::Invalid(format!("{}: {reason}", file.display())))
}

/// Whether `text` has a `[sandbox]` table, read leniently: for the listing of a
/// profile that is not valid. Text that is not TOML has none.
fn has_sandbox_table(text: &str) -> bool {
    text.parse::<toml::Table>()
        .is_ok_and(|table| table.get("sandbox").is_some_and(toml::Value::is_table))
}

/// Load profile `name` from `dir` as a [`Snapshot`], reading the file now (P4).
/// `home` expands `~`.
pub fn load(dir: &Path, name: &str, home: Option<&Path>) -> Result<Snapshot, Error> {
    read(dir, name, home).map(|parsed| parsed.snapshot)
}

/// Cut `text` to at most [`MAX_TEXT`] bytes at a character boundary.
fn cut(mut text: String) -> String {
    if text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Every `*.toml` of `dir` whose stem is a valid name, sorted by name, at most
/// [`MAX_LISTED`] (P6); no directory, no profiles. A directory that is there and
/// cannot be read is an error naming it, not an empty listing.
pub fn list(dir: &Path, home: Option<&Path>) -> Result<Vec<Listed>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("{}: {err}", dir.display())),
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            let stem = file.strip_suffix(".toml")?;
            valid_name(stem).then(|| stem.to_owned())
        })
        .collect();
    names.sort();
    names.truncate(MAX_LISTED);
    Ok(names
        .into_iter()
        .map(|name| {
            let failed = |sandboxed: bool, err: Error, name: String| Listed {
                description: String::new(),
                sandboxed,
                error: cut(err.to_string()),
                name,
            };
            let (file, text) = match read_text(dir, &name, home) {
                Ok(read) => read,
                // Not read: whether it has a [sandbox] table is not known.
                Err(err) => return failed(false, err, name),
            };
            match parse(&name, &text, home) {
                Ok(parsed) => Listed {
                    sandboxed: parsed.snapshot.sandbox.is_some(),
                    description: cut(parsed.description),
                    error: String::new(),
                    name,
                },
                Err(reason) => failed(
                    has_sandbox_table(&text),
                    Error::Invalid(format!("{}: {reason}", file.display())),
                    name,
                ),
            }
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const HOME: &str = "/home/test";

    fn home() -> Option<&'static Path> {
        Some(Path::new(HOME))
    }

    fn parse_ok(text: &str) -> Snapshot {
        parse("demo", text, home()).unwrap().snapshot
    }

    fn parse_err(text: &str) -> String {
        parse("demo", text, home()).err().unwrap()
    }

    #[test]
    fn relative_paths_are_refused() {
        for text in [
            "cwd = \"work\"",
            "cwd = \"./work\"",
            "cwd = \"~other/work\"",
            "[sandbox]\nread_only = [\"docs\"]",
            "[sandbox]\nread_write = [\"/ok\", \"../x\"]",
            "[sandbox]\nexecutables = [\"git\"]",
        ] {
            let err = parse_err(text);
            assert!(
                err.contains("must be absolute or start with ~"),
                "{text}: {err}"
            );
        }
        assert!(parse_err("cwd = \"work\"").contains("cwd"));
        assert!(parse_err("cwd = \"/a\\nb\"").contains("NUL or a newline"));
        assert!(parse_err("cwd = \"/a\\u0000b\"").contains("NUL or a newline"));
    }

    #[test]
    fn tilde_is_the_home_directory() {
        let snapshot = parse_ok(
            "cwd = \"~\"\n[env]\nA = \"~/bin\"\nB = \"x~/y\"\n[sandbox]\nread_only = [\"~/docs/./a/../b\", \"/etc/../usr\"]\nread_write = [\"~/\"]",
        );
        assert_eq!(snapshot.cwd.as_deref(), Some(Path::new(HOME)));
        assert_eq!(snapshot.env["A"], "/home/test/bin");
        assert_eq!(snapshot.env["B"], "x~/y");
        let sandbox = snapshot.sandbox.unwrap();
        assert_eq!(
            sandbox.read_only,
            vec![PathBuf::from("/home/test/docs/b"), PathBuf::from("/usr")]
        );
        assert_eq!(sandbox.read_write, vec![PathBuf::from(HOME)]);
        // Without a home directory `~` is an error, not a guess.
        assert!(
            parse("demo", "cwd = \"~\"", None)
                .err()
                .unwrap()
                .contains("home directory")
        );
        // `..` does not climb out of the root.
        assert_eq!(
            parse_ok("cwd = \"/../../a\"").cwd.as_deref(),
            Some(Path::new("/a"))
        );
    }

    #[test]
    fn tilr_variables_cannot_be_set() {
        let err = parse_err("[env]\nTILR_PANE = \"x\"");
        assert!(err.contains("TILR_PANE"), "{err}");
        assert!(parse_err("[env]\nTILR_ANYTHING = \"x\"").contains("TILR_ANYTHING"));
        assert!(parse_err("[env]\n\"\" = \"x\"").contains("empty"));
        assert!(parse_err("[env]\n\"A=B\" = \"x\"").contains("A=B"));
        assert!(parse_err("[sandbox]\nenv = [\"TILR_X\"]").contains("TILR_X"));
        // Other names, and names that only contain TILR_, are fine.
        let snapshot = parse_ok("[env]\nTILR = \"1\"\nMY_TILR_X = \"2\"");
        assert_eq!(snapshot.env.len(), 2);
    }

    #[test]
    fn every_sandbox_key_is_parsed() {
        let snapshot = parse_ok(
            "description = \"d\"\ncommand = [\"/bin/sh\", \"-c\", \"x\"]\n[sandbox]\nread_only = [\"/a\"]\nread_write = [\"/b\"]\nexecutables = [\"/c\"]\nenv = [\"LANG\", \"EDITOR\"]\nnetwork = \"client\"",
        );
        assert_eq!(snapshot.name, "demo");
        assert_eq!(snapshot.argv, ["/bin/sh", "-c", "x"]);
        assert_eq!(
            snapshot.sandbox,
            Some(SandboxSpec {
                read_only: vec!["/a".into()],
                read_write: vec!["/b".into()],
                executables: vec!["/c".into()],
                env: vec!["LANG".into(), "EDITOR".into()],
                network: Network::Client,
            })
        );
        // An empty table is a sandbox with every default (network none).
        let empty = parse_ok("[sandbox]").sandbox.unwrap();
        assert_eq!(empty, SandboxSpec::default());
        assert_eq!(empty.network, Network::None);
        for network in ["server", "none"] {
            let text = format!("[sandbox]\nnetwork = \"{network}\"");
            assert!(parse_ok(&text).sandbox.is_some());
        }
        // Unknown keys at any level and wrong types make the profile invalid.
        assert!(parse_err("comand = [\"/bin/sh\"]").contains("comand"));
        assert!(parse_err("[sandbox]\nread_onlyy = []").contains("read_onlyy"));
        assert!(parse_err("[sandbox]\nnetwork = \"all\"").contains("all"));
        assert!(parse_err("command = \"/bin/sh\"").contains("command"));
        assert!(
            parse_err("[env]\nA = 1").contains("A") || parse_err("[env]\nA = 1").contains("string")
        );
        assert!(!parse_err("command = [").is_empty());
    }

    #[test]
    fn names_never_are_paths() {
        for ok in ["demo", "a", "A1", "a.b_c-d", &"x".repeat(64)] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".hidden",
            "-x",
            "_x",
            "../server",
            "/etc/passwd",
            "a/b",
            "a b",
            "a\0b",
            "é",
            &"x".repeat(65),
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    /// A directory tree for one test, standing in for the home directory: the check
    /// of the directories above a profile stops there, as at a real home, so the tree
    /// may live in the system's temporary directory (`/tmp` itself would be refused).
    /// Removed when dropped.
    pub(crate) struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        pub(crate) fn new() -> Self {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let root = std::env::temp_dir().join(format!(
                "tilrd-profile-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self { root }
        }

        /// The home directory of the tree.
        pub(crate) fn home(&self) -> Option<&Path> {
            Some(&self.root)
        }

        /// A new directory `name` in the tree, mode 0700.
        pub(crate) fn dir(&self, name: &str) -> PathBuf {
            let dir = self.root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Write `text` into `dir/name` with mode `mode`.
    pub(crate) fn write(dir: &Path, name: &str, text: &str, mode: u32) -> PathBuf {
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        file
    }

    #[test]
    fn only_the_user_or_root_may_own_a_profile_or_a_directory_above_it() {
        let path = Path::new("/p/demo.toml");
        let me = 501;
        assert_eq!(secure(path, me, 0o100600, me), Ok(()));
        assert_eq!(secure(path, me, 0o100644, me), Ok(()));
        assert_eq!(secure(path, 0, 0o040755, me), Ok(()));
        // As root, root's own files.
        assert_eq!(secure(path, 0, 0o100600, 0), Ok(()));
        assert_eq!(
            secure(path, 1000, 0o100600, me),
            Err("/p/demo.toml is owned by uid 1000".to_owned())
        );
        // Root running the server does not accept another user's file either.
        assert!(secure(path, me, 0o100600, 0).is_err());
        for mode in [0o100620, 0o100602, 0o100666, 0o040775, 0o041777] {
            assert_eq!(
                secure(path, me, mode, me),
                Err("/p/demo.toml is writable by others".to_owned()),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_profile_others_can_write_does_not_load() {
        let tree = Scratch::new();
        let dir = tree.dir("profiles");
        let file = write(&dir, "demo.toml", "command = [\"/bin/sh\"]", 0o666);
        let err = load(&dir, "demo", tree.home()).unwrap_err().to_string();
        assert!(err.contains(&file.display().to_string()), "{err}");
        assert!(err.contains("is writable by others"), "{err}");
        // Group write counts too; read for others does not.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o620)).unwrap();
        assert!(load(&dir, "demo", tree.home()).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&dir, "demo", tree.home()).is_ok());
        // The directory too.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = load(&dir, "demo", tree.home()).unwrap_err().to_string();
        assert!(err.contains("is writable by others"), "{err}");
        let real = std::fs::canonicalize(&dir).unwrap();
        assert!(err.contains(&real.display().to_string()), "{err}");
    }

    #[test]
    fn the_directories_above_are_checked_up_to_the_home_directory() {
        let tree = Scratch::new();
        // root/open (group-writable)/home/profiles/demo.toml
        let open = tree.dir("open");
        let home = tree.dir("open/home");
        let dir = tree.dir("open/home/profiles");
        write(&dir, "demo.toml", "", 0o600);
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o770)).unwrap();
        let err = load(&dir, "demo", tree.home()).unwrap_err().to_string();
        let real = std::fs::canonicalize(&open).unwrap();
        assert_eq!(err, format!("{} is writable by others", real.display()));
        // Above the home directory nothing is checked, as in sshd.
        assert!(load(&dir, "demo", Some(&home)).is_ok());
        // The home directory itself is.
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o720)).unwrap();
        let err = load(&dir, "demo", Some(&home)).unwrap_err().to_string();
        let real = std::fs::canonicalize(&home).unwrap();
        assert_eq!(err, format!("{} is writable by others", real.display()));
    }

    #[test]
    fn a_symlinked_profile_is_judged_where_it_lives() {
        let tree = Scratch::new();
        let dir = tree.dir("profiles");
        let other = tree.dir("elsewhere");
        let target = write(&other, "real.toml", "command = [\"/bin/sh\"]", 0o600);
        std::os::unix::fs::symlink(&target, dir.join("demo.toml")).unwrap();
        assert_eq!(
            load(&dir, "demo", tree.home()).unwrap().argv,
            vec!["/bin/sh".to_owned()]
        );
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            load(&dir, "demo", tree.home())
                .unwrap_err()
                .to_string()
                .contains("is writable by others")
        );
        // The directory the link points into counts, not the one the link is in.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o730)).unwrap();
        let err = load(&dir, "demo", tree.home()).unwrap_err().to_string();
        let real = std::fs::canonicalize(&other).unwrap();
        assert_eq!(err, format!("{} is writable by others", real.display()));
        // A link to a directory is not a regular file.
        std::os::unix::fs::symlink(&other, dir.join("dir.toml")).unwrap();
        assert!(
            load(&dir, "dir", tree.home())
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );
    }

    #[test]
    fn a_profile_is_read_up_to_its_bound() {
        let tree = Scratch::new();
        let dir = tree.dir("profiles");
        let bound = usize::try_from(MAX_FILE).unwrap();
        // Exactly the bound: a comment line of that length loads.
        let mut text = "#".repeat(bound - 1);
        text.push('\n');
        write(&dir, "big.toml", &text, 0o600);
        assert!(load(&dir, "big", tree.home()).is_ok());
        text.push('\n');
        let file = write(&dir, "big.toml", &text, 0o600);
        assert_eq!(
            load(&dir, "big", tree.home()).unwrap_err().to_string(),
            format!("{} is larger than 65536 bytes", file.display())
        );
    }

    #[test]
    fn a_missing_profile_or_directory_is_no_such_profile() {
        let tree = Scratch::new();
        let dir = tree.dir("profiles");
        assert_eq!(
            load(&dir, "nope", tree.home()),
            Err(Error::NoSuch("nope".to_owned()))
        );
        assert_eq!(
            load(&dir.join("absent"), "nope", tree.home()),
            Err(Error::NoSuch("nope".to_owned()))
        );
        assert_eq!(
            load(&dir, "../x", tree.home()),
            Err(Error::BadName("../x".to_owned()))
        );
        assert!(list(&dir.join("absent"), tree.home()).unwrap().is_empty());
        // A profiles directory that is there and cannot be read is no empty listing.
        let file = dir.join("not-a-directory");
        std::fs::write(&file, "").unwrap();
        let err = list(&file, tree.home()).unwrap_err();
        assert!(err.starts_with(&file.display().to_string()), "{err}");
    }

    #[test]
    fn the_listing_is_sorted_with_errors_and_skips_other_files() {
        let tree = Scratch::new();
        let dir = tree.dir("profiles");
        write(&dir, "b.toml", "[sandbox]\nread_write = [\"/w\"]", 0o600);
        write(&dir, "a.toml", "description = \"first\"", 0o600);
        write(&dir, "c.toml", "this is not toml", 0o600);
        write(&dir, ".x.toml", "", 0o600);
        write(&dir, "notes.txt", "", 0o600);
        // Invalid for another reason, with a [sandbox] table; and one never read.
        write(&dir, "d.toml", "comand = []\n[sandbox]\n", 0o600);
        write(&dir, "e.toml", "[sandbox]\n", 0o666);
        let listed = list(&dir, tree.home()).unwrap();
        let names: Vec<&str> = listed.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d", "e"]);
        assert_eq!(listed[0].description, "first");
        assert!(listed[0].error.is_empty() && !listed[0].sandboxed);
        assert!(listed[1].sandboxed && listed[1].error.is_empty());
        assert!(!listed[2].error.is_empty() && !listed[2].sandboxed);
        assert!(listed[3].error.contains("comand") && listed[3].sandboxed);
        assert!(listed[4].error.contains("is writable by others") && !listed[4].sandboxed);
        write(
            &dir,
            "f.toml",
            &format!("description = \"{}\"", "é".repeat(2000)),
            0o600,
        );
        let listed = list(&dir, tree.home()).unwrap();
        assert!(listed[5].description.len() <= MAX_TEXT);
    }

    #[test]
    fn only_a_sandbox_table_counts_as_a_sandbox() {
        assert!(has_sandbox_table("[sandbox]\nbad = 1"));
        assert!(has_sandbox_table("x = 1\n[sandbox.inner]"));
        assert!(!has_sandbox_table("sandbox = true"));
        assert!(!has_sandbox_table("[sandbox"));
        assert!(!has_sandbox_table(""));
    }
}
