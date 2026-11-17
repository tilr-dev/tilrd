//! Helpers of `pane-profiles` for the bus tests, beside `bus.rs`.
//!
//! Owner: `pane-profiles`.

#![allow(dead_code, reason = "each test binary uses some of the helpers")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use tilr_bus::pane::{
    CreateRequest, ListReply, PaneInfo, ProfileInfo, ProfilesReply, ProfilesRequest,
};
use tilr_bus::{Bus, BusError, CallOptions};

use super::bus::{BusOptions, BusServer};
pub use super::bus_panes::Sink;

/// A new directory for profile files, in a tree that passes the server's check of the
/// directories above a profile (owned by the user or root, not writable by group or
/// others, up to `/` or the home directory): under Cargo's temporary directory of the
/// target, not the system's (`/tmp`, writable by all, is refused as sshd refuses it).
pub fn secure_dir() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "profiles-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    make_dir(&dir);
    dir
}

/// A server whose configuration directory (`TILR_CONFIG_DIR`, with `server.toml` and
/// the default `profiles/`) is a [`secure_dir`].
pub struct ProfileServer {
    pub server: BusServer,
    /// The configuration directory.
    pub config: PathBuf,
}

impl std::ops::Deref for ProfileServer {
    type Target = BusServer;

    fn deref(&self) -> &BusServer {
        &self.server
    }
}

/// Start a [`ProfileServer`] with `server_toml` as its configuration file.
pub fn start_with(server_toml: &str) -> ProfileServer {
    let config = secure_dir();
    std::fs::write(config.join("server.toml"), server_toml).unwrap();
    let server = BusServer::start(BusOptions {
        env: vec![(
            "TILR_CONFIG_DIR".to_owned(),
            config.to_str().unwrap().to_owned(),
        )],
        ..BusOptions::default()
    });
    ProfileServer { server, config }
}

/// `profiles/` in the configuration directory of the server, where it looks by default.
pub fn default_profiles_dir(server: &ProfileServer) -> PathBuf {
    server.config.join("profiles")
}

/// Make `dir` as a profiles directory is expected to be (owned, mode 0700).
pub fn make_dir(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// Write the profile `<name>.toml` in `dir` with mode `mode`.
pub fn write_in(dir: &Path, name: &str, text: &str, mode: u32) -> PathBuf {
    make_dir(dir);
    let file = dir.join(format!("{name}.toml"));
    std::fs::write(&file, text).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
    file
}

/// Write a valid-mode profile in the default directory of `server`; it is read on the
/// next `pane.create`, the server needs no restart.
pub fn write_profile(server: &ProfileServer, name: &str, text: &str) -> PathBuf {
    write_in(&default_profiles_dir(server), name, text, 0o600)
}

/// `pane.profiles`.
pub async fn profiles(bus: &Bus) -> Vec<ProfileInfo> {
    let reply: ProfilesReply = bus
        .call_typed("pane.profiles", &ProfilesRequest {}, CallOptions::default())
        .await
        .expect("pane.profiles");
    reply.profiles
}

/// `pane.create` with a profile, the request's other fields as given.
pub async fn create(
    bus: &Bus,
    profile: &str,
    command: &[&str],
    cwd: &str,
) -> Result<PaneInfo, BusError> {
    bus.call_typed(
        "pane.create",
        &CreateRequest {
            cols: 80,
            rows: 24,
            command: command.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: cwd.to_owned(),
            profile: profile.to_owned(),
        },
        CallOptions::default(),
    )
    .await
}

/// `pane.list`.
pub async fn list_panes(bus: &Bus) -> Vec<PaneInfo> {
    let reply: ListReply = bus
        .call_typed(
            "pane.list",
            &tilr_bus::pane::ListRequest {},
            CallOptions::default(),
        )
        .await
        .expect("pane.list");
    reply.panes
}

/// A shell command that writes what `script` prints into the FIFO of `sink`, which
/// the test reads as the bytes come.
pub fn write_to(sink: &Sink, script: &str) -> Vec<String> {
    vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("( {script} ) > '{}'", sink.path.display()),
    ]
}
