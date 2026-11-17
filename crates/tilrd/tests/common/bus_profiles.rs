//! Helpers of `pane-profiles` for the bus tests, beside `bus.rs`.
//!
//! Owner: `pane-profiles`.

#![allow(dead_code, reason = "each test binary uses some of the helpers")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tilr_bus::pane::{
    CreateRequest, ListReply, PaneInfo, ProfileInfo, ProfilesReply, ProfilesRequest,
};
use tilr_bus::{Bus, BusError, CallOptions};

use super::TIMEOUT;
use super::bus::BusServer;

/// `profiles/` in the configuration directory of the server, where it looks by default.
pub fn default_profiles_dir(server: &BusServer) -> PathBuf {
    server.dir.join("config/profiles")
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
pub fn write_profile(server: &BusServer, name: &str, text: &str) -> PathBuf {
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

/// A shell command that writes what `script` prints to `out`, atomically.
pub fn write_to(out: &Path, script: &str) -> Vec<String> {
    let tmp = format!("{}.tmp", out.display());
    vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("( {script} ) > '{tmp}' && mv '{tmp}' '{}'", out.display()),
    ]
}

/// Wait for the file `path` and return its content.
pub async fn wait_for_file(path: &Path) -> String {
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{} was not written",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
