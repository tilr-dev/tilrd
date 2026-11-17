//! Sandboxed panes (shared design Part S): the platform-neutral half.
//!
//! A pane of a sandboxed profile runs its program, and everything that program starts,
//! restricted to the grants of the profile's `[sandbox]` table. This module decides
//! what the restriction is made of, the same on every platform (S6-S9): the
//! environment from scratch, the grants refused because they would expose the server
//! (S6), the private `TMPDIR` and its cleanup (S8), the `sandbox` feature check (S9)
//! and failing closed (S7). The platform enforces it: `linux.rs` with Landlock and
//! seccomp (S5); macOS is `sandbox-macos`'s.
//!
//! Nothing here knows of profiles or the bus. The input is a [`Request`], plain data
//! with the meaning of the profile's `[sandbox]` table and `[env]`; the pane process
//! gets it, with the server's [`Dirs`], in a private file its command line names
//! (`tilrd pane --sandbox <file>`, [`Handover`]: no value of `[env]` shows in `ps`) and
//! hands it to `pty::spawn_shell`, which calls
//! [`plan`] before it forks and [`Plan::apply_in_child`] between fork and exec.

#[cfg(target_os = "linux")]
mod linux;

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
use linux::Linux as Current;

/// What a sandboxed pane may do with the network (S5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// No TCP connect or bind, no UDP (default).
    #[default]
    None,
    /// Outgoing TCP connections; no listening. UDP is not restricted on Linux.
    Client,
    /// Incoming and outgoing.
    Server,
}

/// What a sandboxed pane is allowed: the `[sandbox]` table of its profile, resolved,
/// and the profile's `[env]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Absolute paths the pane may read and execute, not write.
    #[serde(default)]
    pub read_only: Vec<PathBuf>,
    /// Absolute paths the pane may read, write and execute.
    #[serde(default)]
    pub read_write: Vec<PathBuf>,
    /// Programs the pane may run besides those under its grants and the system
    /// directories: an absolute path, or a name (no `/`) looked up in the pane's `PATH`
    /// when it is created (the `PATH` of `env` if it sets one, else the server's).
    #[serde(default)]
    pub executables: Vec<PathBuf>,
    /// Names of the server's environment variables the pane gets, with their values.
    #[serde(default)]
    pub pass_env: Vec<String>,
    #[serde(default)]
    pub network: Network,
    /// The profile's `[env]`, set last (S6).
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

/// What the server hands the pane process of a sandboxed pane: the request and the
/// server's directories (with those of a `--config` elsewhere, which the pane process
/// cannot know), in a private file its command line names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handover {
    pub request: Request,
    pub dirs: Dirs,
}

impl Handover {
    /// Hand it to the pane process of pane `pane_id`: write it to its file in the
    /// runtime directory (`<runtime>/sandbox/<pane id>.request`, 0600, in a 0700
    /// directory no grant may cover) and return the arguments that name it,
    /// `--sandbox <file>`. The values of `[env]` stay out of the pane process's command
    /// line, which any process of the user can read (`ps`, `/proc/<pid>/cmdline`). A
    /// reload execs the pane process with the same arguments; the file stays until the
    /// pane ends ([`remove_pane_dir`]).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "pane.create (bus-core-panes) starts sandboxed panes with it"
        )
    )]
    pub fn write(&self, pane_id: &str) -> Result<[OsString; 2], String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        check_pane_id(pane_id)?;
        let runtime = &self.dirs.runtime;
        let parent = runtime.join("sandbox");
        crate::clipboard::private_dir(&parent).map_err(|err| format!("sandbox: {err:#}"))?;
        let path = request_path(runtime, pane_id);
        remove_entry(&path);
        let json = serde_json::to_vec(self).expect("a sandbox request serializes");
        std::fs::File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .and_then(|mut file| file.write_all(&json))
            .map_err(|err| format!("sandbox: write {}: {err}", path.display()))?;
        Ok(["--sandbox".into(), path.into_os_string()])
    }

    /// What the file the pane process's command line names holds.
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path)
            .map_err(|err| format!("sandbox: the request {}: {err}", path.display()))?;
        serde_json::from_slice(&bytes).map_err(|err| format!("sandbox: the request: {err}"))
    }
}

/// The file the request of pane `pane_id` is handed over in.
pub fn request_path(runtime: &Path, pane_id: &str) -> PathBuf {
    runtime.join("sandbox").join(format!("{pane_id}{REQUEST}"))
}

/// The suffix of a request's file beside the per-pane directories.
const REQUEST: &str = ".request";

/// What the start-up check found the platform can enforce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// The Landlock ABI the sandbox is built for (Linux; 0 elsewhere).
    pub landlock_abi: u32,
}

impl Features {
    /// Whether a sandboxed pane is kept from signalling processes outside its sandbox
    /// (Landlock ABI 6 and later).
    pub fn signal_scoping(self) -> bool {
        !cfg!(target_os = "linux") || self.landlock_abi >= 6
    }
}

/// A platform's enforcement of the sandbox. `linux.rs` implements it; `sandbox-macos`
/// adds `macos.rs`.
trait Platform {
    /// What a [`plan`] carries from before the fork into the child.
    type Prepared: Send + Sync + 'static;
    /// The start-up check: what this host can enforce, or why it cannot.
    fn check() -> Result<Features, String>;
    /// Build the restriction for `grants` and `network` in the process that forks.
    /// Every grant with `checked` set is checked by what was opened for it
    /// ([`refuse_opened`]), not by its path again.
    fn prepare(
        grants: &[Grant],
        network: Network,
        features: Features,
        dirs: &Dirs,
    ) -> Result<Self::Prepared, String>;
    /// Install the restriction on the calling process. Runs between fork and exec, so
    /// only async-signal-safe calls; an error makes the spawn fail before the exec.
    fn apply_in_child(prepared: &Self::Prepared) -> io::Result<()>;
}

/// The platform without a sandbox.
#[cfg(not(target_os = "linux"))]
struct Current;

/// Nothing to install where there is no sandbox.
#[cfg(not(target_os = "linux"))]
struct Unsupported;

#[cfg(not(target_os = "linux"))]
impl Platform for Current {
    type Prepared = Unsupported;
    fn check() -> Result<Features, String> {
        Err("this system has no sandbox for panes".into())
    }
    fn prepare(_: &[Grant], _: Network, _: Features, _: &Dirs) -> Result<Unsupported, String> {
        Err("this system has no sandbox for panes".into())
    }
    fn apply_in_child(_: &Unsupported) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }
}

/// How a path is granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    /// Read and execute (`read_only`).
    ReadOnly,
    /// Everything (`read_write`, `TMPDIR`).
    ReadWrite,
    /// Read and execute one program (`executables`, the program the pane runs).
    Program,
    /// Read, write and use the pane's own terminal device.
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Grant {
    path: PathBuf,
    access: Access,
    /// A grant of the request (a grant path, an executable, the program), checked
    /// against the server's directories; not the pane's own `TMPDIR` and terminal.
    checked: bool,
}

/// The start-up check (S9), once per process: `Ok` means `sandbox` is listed in
/// `server.info.features` and sandboxed profiles start. Failpoints: `sandbox-unavailable`
/// makes it fail, `sandbox-abi-4` makes Linux look like Landlock ABI 4.
pub fn features() -> Result<Features, String> {
    static FEATURES: OnceLock<Result<Features, String>> = OnceLock::new();
    FEATURES
        .get_or_init(|| {
            if crate::fail::point("sandbox-unavailable").is_err() {
                return Err("failpoint sandbox-unavailable".into());
            }
            let mut features = Current::check()?;
            if cfg!(target_os = "linux") && crate::fail::point("sandbox-abi-4").is_err() {
                features.landlock_abi = features.landlock_abi.min(4);
            }
            Ok(features)
        })
        .clone()
}

/// At server start (S8, S9): remove what sandboxed panes that no longer run left in
/// the runtime directory, and say whether this server offers the sandbox.
pub fn at_server_start(runtime: &Path, running: impl Fn(&str) -> bool) {
    remove_leftovers(runtime, running);
    match features() {
        Ok(features) => tracing::info!(
            landlock_abi = features.landlock_abi,
            "sandbox: offered (sandboxed profiles start)"
        ),
        Err(reason) => tracing::info!("sandbox: not offered: {reason}"),
    }
}

/// The warning S5 asks for when a pane starts without signal scoping.
fn scoping_warning(features: Features, pane_id: &str) -> Option<String> {
    (!features.signal_scoping()).then(|| {
        format!(
            "sandbox: kernel lacks signal scoping (Landlock ABI {} < 6); pane {pane_id} can \
             signal the user's other processes",
            features.landlock_abi
        )
    })
}

/// Where the user's terminals are: one pane's slave is `/dev/pts/N`.
const TERMINALS: &str = "/dev/pts";

/// The warning when a grant of the profile covers the user's terminals (or one of
/// them): the way to run tmux or `script` inside, at the price of every terminal of the
/// user being open to the pane.
fn terminals_warning(request: &Request, pane_id: &str) -> Option<String> {
    let terminals = Path::new(TERMINALS);
    let grant = request
        .read_only
        .iter()
        .chain(&request.read_write)
        .find(|grant| {
            forms(grant)
                .iter()
                .any(|form| terminals.starts_with(form) || form.starts_with(terminals))
        })?;
    Some(format!(
        "sandbox: the grant {} covers {TERMINALS}: pane {pane_id} can open every terminal \
         of the user, read what is typed there and write to it",
        grant.display()
    ))
}

/// A directory a grant must not expose (S6), and what is in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protected {
    pub path: PathBuf,
    /// Says what the directory holds: "the state directory", say.
    pub what: String,
}

/// The server's directories as the sandbox needs them: where the per-pane directories
/// go, which directories no grant may cover, and which files no `read_write` grant may.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dirs {
    /// The runtime directory; per-pane directories are `<runtime>/sandbox/<pane id>/`.
    pub runtime: PathBuf,
    pub protected: Vec<Protected>,
    /// Paths a grant may read but not write: the server's own executable, which a
    /// reload execs outside the sandbox.
    pub write_protected: Vec<Protected>,
}

impl Dirs {
    /// This server's, from its environment: the runtime directory and the directory of
    /// the socket (sockets), the state directory (token, root key) and the default
    /// configuration directory (`server.toml`, `bus.toml`, profiles); and, from
    /// `read_write` grants, the server's executable. A server started with `--config`
    /// elsewhere or another profiles directory adds those with [`protect_config_file`]
    /// and [`protect_profiles_dir`].
    ///
    /// [`protect_config_file`]: Dirs::protect_config_file
    /// [`protect_profiles_dir`]: Dirs::protect_profiles_dir
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "pane.create (bus-core-panes) hands them to the pane process"
        )
    )]
    pub fn current() -> Self {
        let runtime = crate::clipboard::runtime_dir();
        let mut dirs = Dirs {
            runtime: runtime.clone(),
            protected: Vec::new(),
            write_protected: Vec::new(),
        };
        if let Some(exe) = own_executable() {
            dirs.write_protected.push(Protected {
                path: exe,
                what: "the server's executable".into(),
            });
        }
        dirs.protect(runtime, "the runtime directory (sockets)");
        let socket = tilr_proto::local::socket_path();
        if let Some(parent) = socket.parent() {
            dirs.protect(parent.to_path_buf(), "the directory of the server's socket");
        }
        let host = crate::host::runtime_socket(Path::new("x.sock"));
        if let Some(parent) = host.parent() {
            dirs.protect(
                parent.to_path_buf(),
                "the directory of the server's sockets",
            );
        }
        if let Some(state) = crate::identity::state_dir() {
            dirs.protect(state, "the state directory (token, root key)");
        }
        if let Some(config) = crate::config::default_path()
            .as_deref()
            .and_then(Path::parent)
        {
            dirs.protect(
                config.to_path_buf(),
                "the configuration directory (profiles, bus.toml)",
            );
        }
        dirs
    }

    /// Add a directory no grant may cover.
    pub fn protect(&mut self, path: PathBuf, what: &str) {
        if !self.protected.iter().any(|p| p.path == path) {
            self.protected.push(Protected {
                path,
                what: what.into(),
            });
        }
    }

    /// The server's configuration file, given with `--config`: its directory holds
    /// `server.toml`, `bus.toml` and the profiles beside it. A relative path is taken
    /// from the current directory.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "pane.create (pane-profiles) passes the server's --config"
        )
    )]
    pub fn protect_config_file(&mut self, file: &Path) {
        let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        if let Some(dir) = file.parent() {
            self.protect(dir.to_path_buf(), "the configuration directory (--config)");
        }
    }

    /// The directory the server reads launch profiles from, where it is not the
    /// configuration directory.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "pane.create (pane-profiles) passes the profiles directory"
        )
    )]
    pub fn protect_profiles_dir(&mut self, dir: &Path) {
        let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        self.protect(dir, "the profiles directory");
    }
}

/// This process's executable, with links resolved: the server's (the pane process is
/// the same file). `None` where it cannot be told.
fn own_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // An executable replaced on disk (an update) reads as `<path> (deleted)`; the path
    // is what a reload execs.
    let exe = match exe.to_str().and_then(|exe| exe.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => exe,
    };
    std::fs::canonicalize(&exe).ok().or(Some(exe))
}

/// `path` with `.` and `..` folded lexically, absolute.
fn folded(path: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    out
}

/// The forms a path is compared in: as written (folded) and, where it exists, with
/// symlinks resolved.
fn forms(path: &Path) -> Vec<PathBuf> {
    let mut forms = vec![folded(path)];
    if let Ok(real) = std::fs::canonicalize(path)
        && !forms.contains(&real)
    {
        forms.push(real);
    }
    forms
}

/// The protected paths with the forms they are compared in: every protected directory,
/// and for a grant that writes the write-protected files too.
fn guarded(dirs: &Dirs, write: bool) -> Vec<(Vec<PathBuf>, &Protected)> {
    let write_protected = dirs.write_protected.iter().filter(|_| write);
    dirs.protected
        .iter()
        .chain(write_protected)
        .map(|p| (forms(&p.path), p))
        .collect()
}

/// The protected path a grant at `form` would cover or lie inside.
fn exposed<'a>(form: &Path, guarded: &[(Vec<PathBuf>, &'a Protected)]) -> Option<&'a Protected> {
    guarded
        .iter()
        .find(|(paths, _)| {
            paths
                .iter()
                .any(|dir| dir.starts_with(form) || form.starts_with(dir))
        })
        .map(|(_, p)| *p)
}

fn exposing(grant: &Path, p: &Protected) -> String {
    format!(
        "sandbox: {} would expose {} {}",
        grant.display(),
        p.what,
        p.path.display()
    )
}

/// Refuse a grant that covers a protected directory, or lies inside one (S6): Landlock
/// grants by allow list only, so a grant of `~` cannot carve the state directory out.
/// A grant that writes (`true`) must not cover the server's executable either.
fn refuse_exposing<'a>(
    grants: impl IntoIterator<Item = (&'a Path, bool)>,
    dirs: &Dirs,
) -> Result<(), String> {
    let (read, write) = (guarded(dirs, false), guarded(dirs, true));
    for (grant, writes) in grants {
        if !grant.is_absolute() {
            return Err(format!(
                "sandbox: {} is not an absolute path",
                grant.display()
            ));
        }
        let guarded = if writes { &write } else { &read };
        for form in forms(grant) {
            if let Some(p) = exposed(&form, guarded) {
                return Err(exposing(grant, p));
            }
        }
    }
    Ok(())
}

/// Refuse a grant by what was opened for it, where the platform opens grants itself
/// (Linux): `opened` is the kernel's path of the descriptor and `id` its device and
/// inode, so a path changed after [`refuse_exposing`] looked at it is caught here.
#[cfg_attr(
    not(target_os = "linux"),
    expect(dead_code, reason = "Linux opens grants")
)]
fn refuse_opened(
    grant: &Path,
    opened: &Path,
    id: (u64, u64),
    write: bool,
    dirs: &Dirs,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let guarded = guarded(dirs, write);
    if let Some(p) = exposed(opened, &guarded) {
        return Err(exposing(grant, p));
    }
    // The same directory mounted elsewhere too.
    for (_, p) in &guarded {
        if std::fs::metadata(&p.path).is_ok_and(|meta| (meta.dev(), meta.ino()) == id) {
            return Err(exposing(grant, p));
        }
    }
    Ok(())
}

/// Whether `path` is under (or is) a protected directory.
fn is_protected(path: &Path, dirs: &Dirs) -> bool {
    refuse_exposing([(path, false)], dirs).is_err()
}

/// The per-pane directory (`TMPDIR`) of pane `pane_id`.
pub fn pane_dir(runtime: &Path, pane_id: &str) -> PathBuf {
    runtime.join("sandbox").join(pane_id)
}

fn check_pane_id(pane_id: &str) -> Result<(), String> {
    if pane_id.is_empty() || pane_id.contains('/') || pane_id.starts_with('.') {
        return Err(format!("sandbox: bad pane id {pane_id:?}"));
    }
    Ok(())
}

/// Make the pane's directory, 0700, empty.
fn create_pane_dir(runtime: &Path, pane_id: &str) -> Result<PathBuf, String> {
    check_pane_id(pane_id)?;
    let parent = runtime.join("sandbox");
    crate::clipboard::private_dir(&parent).map_err(|err| format!("sandbox: {err:#}"))?;
    let dir = pane_dir(runtime, pane_id);
    remove_entry(&dir);
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|err| format!("sandbox: create {}: {err}", dir.display()))?;
    Ok(dir)
}

/// Remove a directory with what is in it, or a file; a link is removed, not followed.
fn remove_entry(path: &Path) {
    let removed = match path.symlink_metadata() {
        Err(err) => Err(err),
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    };
    if let Err(err) = removed
        && err.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("sandbox: remove {}: {err}", path.display());
    }
}

/// Remove what the sandbox made for pane `pane_id` (S8), when the pane ends: its
/// directory and the file its request was handed over in.
pub fn remove_pane_dir(runtime: &Path, pane_id: &str) {
    if check_pane_id(pane_id).is_ok() {
        remove_entry(&pane_dir(runtime, pane_id));
        remove_entry(&request_path(runtime, pane_id));
    }
}

/// Remove the per-pane directories of panes that do not run (S8): at server start,
/// with `running` telling the panes this server has.
pub fn remove_leftovers(runtime: &Path, running: impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(runtime.join("sandbox")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let id = name.strip_suffix(REQUEST).unwrap_or(name);
        if !running(id) {
            tracing::info!("sandbox: removing the leftovers of pane {id}");
            remove_entry(&entry.path());
        }
    }
}

/// Variables a sandboxed pane never gets from the server, whatever `pass_env` names
/// (S6): the server's own (`TILR_*`: shims, socket, bus) and the user's SSH agent.
fn never_passed(name: &str) -> bool {
    name.starts_with("TILR_") || name == "SSH_AUTH_SOCK" || name == "SSH_AGENT_PID"
}

/// The directories of the system's programs, last on a sandboxed pane's default `PATH`.
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// `name` in the directories of `path`, skipping those the pane may not see.
fn lookup(name: &OsStr, path: &OsStr, dirs: &Dirs) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute() && !is_protected(dir, dirs))
        .map(|dir| dir.join(name))
        .find(|file| {
            std::fs::metadata(file)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// What the pane process knows of the pane it starts, for [`plan`].
pub struct Start<'a> {
    pub pane_id: &'a str,
    /// The program the pane runs: the command's first word, or the login shell.
    pub program: &'a str,
    /// The variables every sandboxed pane gets (`HOME`, `USER`, `LOGNAME`, `SHELL`,
    /// `TERM`, `COLORTERM`, `TILR_PANE`), in order.
    pub base_env: Vec<(String, OsString)>,
    /// The server's environment, for `pass_env` and the lookup of `executables`.
    pub server_env: &'a dyn Fn(&str) -> Option<OsString>,
    /// The pane's terminal (`/dev/pts/N`): the only terminal device the pane may open
    /// by name besides `/dev/tty` and `/dev/ptmx`.
    pub terminal: Option<PathBuf>,
}

/// Everything a sandboxed pane is started with, made before the fork.
pub struct Plan {
    /// The whole environment of the program (S6).
    pub env: Vec<(OsString, OsString)>,
    /// The program, resolved through the pane's `PATH`.
    pub program: PathBuf,
    /// The pane's private `TMPDIR` (S8), removed when the pane ends.
    pub tmpdir: PathBuf,
    /// The lines to log for this pane: where its sandbox is weaker than it looks (S5).
    pub warnings: Vec<String>,
    prepared: <Current as Platform>::Prepared,
    /// `sandbox-apply`: installing the restriction fails in the child.
    fail_apply: bool,
}

/// The environment of a sandboxed pane (S6): `base`, `TMPDIR`, `PATH` (the
/// executables' directories, then the system's), the `pass_env` names with the
/// server's values, then the profile's `[env]`.
fn environment(
    request: &Request,
    base: &[(String, OsString)],
    tmpdir: &Path,
    executables: &[PathBuf],
    server_env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    let mut set = |name: &str, value: OsString| {
        env.retain(|(n, _)| n != name);
        env.push((name.into(), value));
    };
    for (name, value) in base {
        set(name, value.clone());
    }
    set("TMPDIR", tmpdir.as_os_str().to_owned());
    let mut path: Vec<PathBuf> = Vec::new();
    for dir in executables.iter().filter_map(|exe| exe.parent()) {
        if !path.iter().any(|known| known == dir) {
            path.push(dir.to_path_buf());
        }
    }
    let mut default_path = std::env::join_paths(&path).unwrap_or_default();
    if !default_path.is_empty() {
        default_path.push(":");
    }
    default_path.push(SYSTEM_PATH);
    set("PATH", default_path);
    for name in &request.pass_env {
        if never_passed(name) {
            tracing::warn!("sandbox: {name} is never passed to a sandboxed pane");
            continue;
        }
        if let Some(value) = server_env(name) {
            set(name, value);
        }
    }
    for (name, value) in &request.env {
        if never_passed(name) {
            tracing::warn!("sandbox: {name} is never set in a sandboxed pane");
            continue;
        }
        set(name, value.into());
    }
    env
}

/// Make the sandbox of a pane: check the grants (S6), resolve the executables and the
/// program, create the private `TMPDIR`, build the environment and the platform's
/// restriction. Every error starts with `sandbox: ` and leaves nothing behind.
pub fn plan(request: &Request, start: &Start<'_>, dirs: &Dirs) -> Result<Plan, String> {
    let features = features().map_err(|err| format!("sandbox: {err}"))?;
    #[cfg(test)]
    let features = Features {
        landlock_abi: tests::ABI.get().unwrap_or(features.landlock_abi),
    };
    plan_for(request, start, dirs, features)
}

fn plan_for(
    request: &Request,
    start: &Start<'_>,
    dirs: &Dirs,
    features: Features,
) -> Result<Plan, String> {
    refuse_exposing(
        request
            .read_only
            .iter()
            .map(|path| (path.as_path(), false))
            .chain(request.read_write.iter().map(|path| (path.as_path(), true))),
        dirs,
    )?;
    let lookup_path = request
        .env
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| OsString::from(value))
        .or_else(|| (start.server_env)("PATH"))
        .unwrap_or_default();
    let mut executables = Vec::new();
    for exe in &request.executables {
        let found = if exe.components().count() == 1 && !exe.is_absolute() {
            lookup(exe.as_os_str(), &lookup_path, dirs)
        } else if exe.is_absolute() {
            Some(exe.clone())
        } else {
            return Err(format!(
                "sandbox: executable {} is neither a name nor an absolute path",
                exe.display()
            ));
        };
        match found {
            Some(found) => executables.push(found),
            None => tracing::info!("sandbox: executable {} not found, skipped", exe.display()),
        }
    }
    refuse_exposing(executables.iter().map(|path| (path.as_path(), false)), dirs)?;

    let tmpdir = create_pane_dir(&dirs.runtime, start.pane_id)?;
    let result = (|| {
        let env = environment(
            request,
            &start.base_env,
            &tmpdir,
            &executables,
            start.server_env,
        );
        let pane_path = env
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let program = Path::new(start.program);
        let program = if program.is_absolute() {
            program.to_path_buf()
        } else if program.components().count() == 1 {
            lookup(program.as_os_str(), &pane_path, dirs).ok_or_else(|| {
                format!("sandbox: {} is not found in the pane's PATH", start.program)
            })?
        } else {
            return Err(format!(
                "sandbox: the program {} is neither a name nor an absolute path",
                start.program
            ));
        };
        refuse_exposing([(program.as_path(), false)], dirs)?;
        let mut grants: Vec<Grant> = Vec::new();
        let mut grant = |path: &Path, access, checked| {
            grants.push(Grant {
                path: path.to_path_buf(),
                access,
                checked,
            })
        };
        for path in &request.read_only {
            grant(path, Access::ReadOnly, true);
        }
        for path in &request.read_write {
            grant(path, Access::ReadWrite, true);
        }
        for path in executables.iter().chain([&program]) {
            grant(path, Access::Program, true);
        }
        grant(&tmpdir, Access::ReadWrite, false);
        if let Some(terminal) = &start.terminal {
            grant(terminal, Access::Terminal, false);
        }
        let prepared =
            Current::prepare(&grants, request.network, features, dirs).map_err(|err| {
                if err.starts_with("sandbox: ") {
                    err
                } else {
                    format!("sandbox: {err}")
                }
            })?;
        let warnings = [
            scoping_warning(features, start.pane_id),
            terminals_warning(request, start.pane_id),
        ];
        Ok(Plan {
            env,
            program,
            tmpdir: tmpdir.clone(),
            warnings: warnings.into_iter().flatten().collect(),
            prepared,
            fail_apply: crate::fail::point("sandbox-apply").is_err() || tests_fail_apply(),
        })
    })();
    if result.is_err() {
        remove_entry(&tmpdir);
    }
    result
}

impl Plan {
    /// Install the restriction on this process: in the child, between fork and exec.
    pub fn apply_in_child(&self) -> io::Result<()> {
        if self.fail_apply {
            return Err(io::Error::from_raw_os_error(libc::EPERM));
        }
        Current::apply_in_child(&self.prepared)
    }
}

/// A test's stand-in for the failpoint `sandbox-apply`, on its own thread.
#[cfg(test)]
fn tests_fail_apply() -> bool {
    tests::FAIL_APPLY.get()
}

#[cfg(not(test))]
fn tests_fail_apply() -> bool {
    false
}

#[cfg(test)]
mod tests;
