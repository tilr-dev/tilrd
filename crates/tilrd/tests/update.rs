//! Use cases: `tilrd update` installs the latest release, tilrd and tilr.

mod common;
#[path = "common/version.rs"]
mod version;
use version::version_and_platform;
#[path = "common/manifest.rs"]
mod manifest;

#[path = "common/cdn.rs"]
mod cdn;
use cdn::serve_files;

use common::*;
use sha2::{Digest, Sha256};

/// `<file>.<extension>`, next to `file`.
fn twin(file: &Path, extension: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(format!(".{extension}"));
    name.into()
}

/// A release archive holding `members` (file name, content), as
/// scripts/release.sh packs one.
fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (name, content) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, name, *content).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// What stands in for the client in a release: it runs.
const CLIENT: &[u8] = b"#!/bin/sh\necho tilr\n";

/// What stands in for the server in a release no test reloads into: the
/// update only checks that `--version` exits 0, so a stub spares packing
/// the real binary.
const STUB: &[u8] = b"#!/bin/sh\nexit 0\n";

/// A program that runs but fails.
const BROKEN: &[u8] = b"#!/bin/sh\nexit 1\n";

/// A release of the server `server` and the stand-in client.
fn release(server: &[u8]) -> Vec<u8> {
    archive(&[("tilrd", server), ("tilr", CLIENT)])
}

/// Where release `version` for `platform` lies under `cdn`.
fn archive_path(cdn: &Path, platform: &str, version: &str) -> PathBuf {
    cdn.join("releases")
        .join(version)
        .join(format!("tilr-{version}-{platform}.tar.gz"))
}

/// Lay out `archive` as release `version` for `platform` under `cdn`, with
/// the checksum of `checksum_of`, and name it the latest one.
fn publish(cdn: &Path, platform: &str, version: &str, archive: &[u8], checksum_of: &[u8]) {
    let path = archive_path(cdn, platform, version);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, archive).unwrap();
    let checksum: String = Sha256::digest(checksum_of)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    manifest::write_manifest(cdn, platform, version, "latest", &checksum);
    let name = path.file_name().unwrap().to_string_lossy();
    std::fs::write(twin(&path, "sha256"), format!("{checksum}  {name}\n")).unwrap();
    let latest = cdn.join("releases").join(platform);
    std::fs::create_dir_all(&latest).unwrap();
    std::fs::write(latest.join("latest"), format!("{version}\n")).unwrap();
}

/// A copy of the server in a directory of its own, and that directory.
fn installed() -> (PathBuf, PathBuf) {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilrd");
    link_executable(&exe);
    (dir, exe)
}

#[test]
fn update_installs_the_latest_release_with_a_legacy_app_marker() {
    let (dir, exe) = installed();
    let marker = dir.join("app-managed.json");
    std::fs::write(&marker, "{}").unwrap();
    let (_, platform) = version_and_platform();
    let archive = release(STUB);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let cdn = serve_files(cdn);

    let output = update(&exe, &cdn);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("-> 9.9.9-test"));
    assert_eq!(std::fs::read(&exe).unwrap(), STUB);
    assert_eq!(std::fs::read(dir.join("tilr")).unwrap(), CLIENT);
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "{}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_selects_beta_without_changing_the_stable_default() {
    for channel in [None, Some(""), Some("stable"), Some("beta")] {
        let (dir, exe) = installed();
        let (version, platform) = version_and_platform();
        let cdn = dir.join("cdn");
        let archive = release(STUB);
        publish(&cdn, &platform, "9.9.9-rc.1-test", &archive, &archive);
        let pointers = cdn.join("releases").join(&platform);
        std::fs::rename(pointers.join("latest"), pointers.join("beta")).unwrap();
        let checksum: String = Sha256::digest(&archive)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        manifest::write_manifest(&cdn, &platform, "9.9.9-rc.1-test", "beta", &checksum);
        manifest::write_manifest(&cdn, &platform, &version, "latest", &"0".repeat(64));
        std::fs::write(pointers.join("latest"), version).unwrap();
        let url = serve_files(cdn);
        let before = std::fs::read(&exe).unwrap();
        let mut cmd = configured_update(&exe, channel);
        cmd.args(["update", "--cdn", &url])
            .env("TILR_CHANNEL", "beta");
        let output = spawn_copied(cmd.stdout(Stdio::piped()).stderr(Stdio::piped()))
            .wait_with_output()
            .unwrap();
        assert!(output.status.success(), "{channel:?}: {output:?}");
        if channel == Some("beta") {
            assert!(String::from_utf8_lossy(&output.stdout).contains("-> 9.9.9-rc.1-test"));
            assert_eq!(std::fs::read(&exe).unwrap(), STUB);
            assert_eq!(std::fs::read(dir.join("tilr")).unwrap(), CLIENT);
        } else {
            assert_eq!(std::fs::read(&exe).unwrap(), before);
            assert!(!dir.join("tilr").exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn configured_beta_stays_current_when_stable_is_older() {
    let (dir, exe) = installed();
    let (version, platform) = version_and_platform();
    let root = dir.join("cdn");
    let archive = release(STUB);
    publish(&root, &platform, "0.0.1-old-stable", &archive, &archive);
    manifest::write_manifest(&root, &platform, &version, "beta", &"0".repeat(64));
    std::fs::write(root.join("releases").join(&platform).join("beta"), &version).unwrap();
    let (url, requests) = cdn::serve_files_with_requests(root);
    let before = std::fs::read(&exe).unwrap();
    let config = dir.join("server.toml");
    std::fs::write(&config, "update_channel = \"beta\"\n").unwrap();
    let output = spawn_copied(
        configured_update(&exe, None)
            .arg("--config")
            .arg(config)
            .args(["update", "--cdn", &url])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .wait_with_output()
    .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("is the latest release"));
    assert_eq!(
        *requests.lock().unwrap(),
        vec![
            format!("/releases/{platform}/beta"),
            format!("/releases/{platform}/beta.manifest.json"),
        ]
    );
    assert_eq!(std::fs::read(&exe).unwrap(), before);
    assert!(!dir.join("tilr").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_rejects_invalid_channels_and_does_not_fall_back_from_beta() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let cdn = dir.join("cdn");
    let archive = release(STUB);
    publish(&cdn, &platform, "9.9.9-stable", &archive, &archive);
    let (url, requests) = cdn::serve_files_with_requests(cdn);
    let before = std::fs::read(&exe).unwrap();
    for channel in ["beta", "rc", "../latest"] {
        requests.lock().unwrap().clear();
        let output = spawn_copied(
            configured_update(&exe, Some(channel))
                .args(["update", "--cdn", &url])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .wait_with_output()
        .unwrap();
        assert!(!output.status.success(), "{channel}: {output:?}");
        if channel != "beta" {
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains("update_channel"), "{error}");
            assert!(error.contains("expected `stable` or `beta`"), "{error}");
            assert!(requests.lock().unwrap().is_empty());
        }
        assert_eq!(std::fs::read(&exe).unwrap(), before);
        assert!(!dir.join("tilr").exists());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Use an isolated default config path, including the absent-file case.
fn configured_update(exe: &Path, channel: Option<&str>) -> Command {
    let config_dir = exe.parent().unwrap().join("config");
    if let Some(channel) = channel {
        std::fs::create_dir_all(&config_dir).unwrap();
        let text = if channel.is_empty() {
            String::new()
        } else {
            format!("update_channel = {channel:?}\n")
        };
        std::fs::write(config_dir.join("server.toml"), text).unwrap();
    }
    let mut cmd = Command::new(exe);
    cmd.env_remove("TILR_CONFIG")
        .env("TILR_CONFIG_DIR", config_dir);
    cmd
}

#[test]
fn update_rejects_the_removed_channel_flag_before_contacting_cdn() {
    let (dir, exe) = installed();
    let (url, requests) = cdn::serve_files_with_requests(dir.join("cdn"));
    let before = std::fs::read(&exe).unwrap();
    let output = spawn_copied(
        configured_update(&exe, None)
            .args(["update", "--cdn", &url, "--channel", "beta"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .wait_with_output()
    .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--channel"));
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(&exe).unwrap(), before);
    assert!(!dir.join("tilr").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

fn update(exe: &Path, cdn: &str) -> std::process::Output {
    update_failing(exe, cdn, "").wait_with_output().unwrap()
}

/// Start an update in which things go wrong where `failpoints` says, see
/// `src/fail.rs`.
fn update_failing(exe: &Path, cdn: &str, failpoints: &str) -> Child {
    spawn_copied(
        configured_update(exe, None)
            .env_remove("RUST_LOG")
            .env("TILR_UPDATE_PUBLIC_KEY", "00".repeat(32))
            .args(["update", "--cdn", cdn])
            .env("TILR_FAILPOINTS", failpoints)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
}

/// This build with a byte more: another file, which runs the same.
fn new_version(exe: &Path) -> Vec<u8> {
    let mut release = std::fs::read(exe).unwrap();
    release.push(0);
    release
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn update_installs_the_latest_release_and_the_running_server_keeps_its_shells() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let server = new_version(&exe);
    let archive = release(&server);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let cdn = serve_files(cdn);

    let running = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&running).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "KEPT=yes").await, 0);

    let output = update(&exe, &cdn);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("-> 9.9.9-test"));
    assert_eq!(std::fs::read(&exe).unwrap(), server);
    assert_eq!(std::fs::read(dir.join("tilr")).unwrap(), CLIENT);

    // The updater explicitly requests reload after the final rename.
    let mut client = reconnect(&running, client).await;
    running.wait_logged("SIGUSR1 came: reload requested").await;
    client.subscribe(pane_id).await;
    assert_eq!(client.run(pane_id, "test \"$KEPT\" = yes").await, 0);
}

#[tokio::test]
async fn update_does_nothing_when_the_latest_release_is_the_one_installed() {
    let (dir, exe) = installed();
    let (version, platform) = version_and_platform();
    let cdn = dir.join("cdn");
    // Authenticated metadata without archive files: nothing to download.
    manifest::write_manifest(&cdn, &platform, &version, "latest", &"0".repeat(64));
    std::fs::create_dir_all(cdn.join("releases").join(&platform)).unwrap();
    std::fs::write(
        cdn.join("releases").join(&platform).join("latest"),
        &version,
    )
    .unwrap();
    let before = std::fs::metadata(&exe).unwrap().modified().unwrap();

    let output = update(&exe, &serve_files(cdn));
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("is the latest release"));
    assert_eq!(std::fs::metadata(&exe).unwrap().modified().unwrap(), before);
}

#[tokio::test]
async fn update_installs_nothing_that_fails_its_checksum_or_does_not_run() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let good = release(STUB);
    let cdn = dir.join("cdn");
    let url = serve_files(cdn.clone());
    let fails = |archive: &[u8], checksum_of: &[u8], said: &str| {
        publish(&cdn, &platform, "9.9.9-test", archive, checksum_of);
        let output = update(&exe, &url);
        assert!(!output.status.success(), "{said}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(said), "{said}: {stderr}");
    };

    // Not the file the checksum was made of.
    fails(&good, b"released", "does not match its checksum");
    // Intact, but not an archive.
    fails(b"not an archive", b"not an archive", "unpack");
    // An archive cut short, its checksum made of what is left.
    let cut = &good[..good.len() / 2];
    fails(cut, cut, "unpack");
    // Either program missing.
    let only_server = archive(&[("tilrd", STUB)]);
    fails(&only_server, &only_server, "the archive has no tilr");
    let only_client = archive(&[("tilr", CLIENT)]);
    fails(&only_client, &only_client, "the archive has no tilrd");
    // Either program does not run: one fails, the other is not a program at all.
    let broken = release(BROKEN);
    fails(
        &broken,
        &broken,
        "tilrd 9.9.9-test does not run on this system",
    );
    let broken = archive(&[("tilrd", STUB), ("tilr", b"not a program")]);
    fails(
        &broken,
        &broken,
        "tilr 9.9.9-test does not run on this system",
    );

    // No release for this platform at all.
    std::fs::remove_dir_all(cdn.join("releases").join(&platform)).unwrap();
    assert!(!update(&exe, &url).status.success());

    // The installed executable is untouched, and nothing is left next to it.
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir), ["cdn", "tilrd"]);
}

#[tokio::test]
async fn update_installs_nothing_when_the_cdn_misbehaves() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let archive = release(STUB);
    let cdn = dir.join("cdn");
    let url = serve_files(cdn.clone());
    let latest = cdn.join("releases").join(&platform).join("latest");
    let file = archive_path(&cdn, &platform, "9.9.9-test");

    // The download breaks off halfway.
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    std::fs::write(twin(&file, "cut"), "").unwrap();
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("download"));

    // The release is named before its files are there.
    std::fs::remove_file(&file).unwrap();
    assert!(!update(&exe, &url).status.success());
    std::fs::remove_file(twin(&file, "sha256")).unwrap();
    std::fs::remove_file(latest.with_file_name("latest.manifest.json")).unwrap();
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("has no release 9.9.9-test"));

    // What answers is not the CDN: a login page, say.
    std::fs::write(&latest, "<html><body>Sign in</body></html>").unwrap();
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a version"));

    // Nobody answers at all.
    assert!(!update(&exe, "http://127.0.0.1:9").status.success());

    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir), ["cdn", "tilrd"]);
}

#[tokio::test]
async fn update_installs_nothing_when_it_cannot_write_next_to_the_executable() {
    use std::os::unix::fs::PermissionsExt;
    // Nothing is closed to root, which the tests are in a container.
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let scratch = Server::scratch_dir();
    let dir = scratch.join("bin");
    std::fs::create_dir(&dir).unwrap();
    let exe = dir.join("tilrd");
    link_executable(&exe);
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let archive = release(STUB);
    let cdn = scratch.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let url = serve_files(cdn);

    std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o555)).unwrap();
    let output = update(&exe, &url);
    std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o755)).unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("write"));
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir), ["tilrd"]);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_update_killed_before_the_swap_changes_nothing_and_the_next_one_tidies_up() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let archive = release(STUB);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let url = serve_files(cdn);

    // Killed with the release unpacked, a moment before it is renamed.
    let killed = update_failing(&exe, &url, "update-before-rename=kill")
        .wait_with_output()
        .unwrap();
    assert!(!killed.status.success());
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(
        files_in(&dir).len(),
        3,
        "the unpacked release is left behind"
    );

    let output = update(&exe, &url);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), STUB);
    assert_eq!(files_in(&dir), ["cdn", "tilr", "tilrd"]);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn a_server_that_cannot_be_put_in_place_puts_the_old_client_back() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let old_client = b"#!/bin/sh\necho old tilr\n";
    std::fs::write(dir.join("tilr"), old_client).unwrap();
    let archive = release(STUB);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let url = serve_files(cdn);

    // The client is in place, the server's rename fails.
    let output = update_failing(&exe, &url, "update-between-renames=error")
        .wait_with_output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nothing was installed"));
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(std::fs::read(dir.join("tilr")).unwrap(), old_client);
    assert_eq!(files_in(&dir), ["cdn", "tilr", "tilrd"]);

    // Without a client before, none is left.
    std::fs::remove_file(dir.join("tilr")).unwrap();
    let output = update_failing(&exe, &url, "update-between-renames=error")
        .wait_with_output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(files_in(&dir), ["cdn", "tilrd"]);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn two_updates_at_once_install_the_release_once() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let archive = release(STUB);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let url = serve_files(cdn);

    // The first is slow to swap, so the second arrives while it is at work.
    let first = update_failing(&exe, &url, "update-before-rename=sleep:1500");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let second = update_failing(&exe, &url, "");
    let outputs = [first, second].map(|update| update.wait_with_output().unwrap());

    let said = |text: &str| {
        outputs
            .iter()
            .filter(|output| String::from_utf8_lossy(&output.stdout).contains(text))
            .count()
    };
    assert!(
        outputs.iter().all(|output| output.status.success()),
        "{outputs:?}"
    );
    assert_eq!(said("-> 9.9.9-test"), 1, "{outputs:?}");
    assert_eq!(said("is already installed"), 1, "{outputs:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), STUB);
    assert_eq!(files_in(&dir), ["cdn", "tilr", "tilrd"]);
}

#[test]
fn fetch_release_writes_a_checked_archive() {
    let dir = Server::scratch_dir();
    // Foreign platform programs must be validated as archive members, never run.
    let bytes = archive(&[("tilr", b"foreign client"), ("tilrd", b"foreign server")]);
    for platform in ["linux-aarch64", "macos-aarch64", "macos-x86_64"] {
        publish(&dir.join("cdn"), platform, "9.9.9", &bytes, &bytes);
        let cdn = serve_files(dir.join("cdn"));
        let out = dir.join(platform);
        let result = Command::new(env!("CARGO_BIN_EXE_tilrd"))
            .args([
                "fetch-release",
                "--platform",
                platform,
                "--cdn",
                &cdn,
                "--out",
            ])
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            std::fs::read(out.join(format!("tilr-9.9.9-{platform}.tar.gz"))).unwrap(),
            bytes
        );
        assert_eq!(std::fs::read_dir(out).unwrap().count(), 1);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fetch_release_refuses_a_bad_checksum() {
    let dir = Server::scratch_dir();
    publish(
        &dir.join("cdn"),
        "linux-aarch64",
        "9.9.9",
        &release(b"foreign"),
        b"wrong",
    );
    let cdn = serve_files(dir.join("cdn"));
    let out = dir.join("out");
    let result = Command::new(env!("CARGO_BIN_EXE_tilrd"))
        .args([
            "fetch-release",
            "--platform",
            "linux-aarch64",
            "--cdn",
            &cdn,
            "--out",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let err = String::from_utf8_lossy(&result.stderr);
    assert!(err.contains("checksum"), "{err}");
    assert!(!err.contains("no release"), "{err}");
    assert!(!err.contains("--archive"), "{err}");
    assert!(!out.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_failed_reload_request_keeps_the_installed_release() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let cdn = Server::scratch_dir();
    let archive = release(STUB);
    publish(&cdn, &platform, "9.9.9-test", &archive, &archive);
    let url = serve_files(cdn.clone());
    let empty_path = Server::scratch_dir();
    // macOS discovery cannot start ps, so the request fails before signalling.
    let output = Command::new(&exe)
        .args(["update", "--cdn", &url])
        .env("PATH", &empty_path)
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("release installed, but could not request reload"),
        "{output:?}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("a running server reloads itself"));
    assert_eq!(std::fs::read(&exe).unwrap(), STUB);
    assert_eq!(std::fs::read(dir.join("tilr")).unwrap(), CLIENT);
    assert_eq!(files_in(&dir), ["tilr", "tilrd"]);
    for path in [dir, cdn, empty_path] {
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn update_requires_authentic_metadata_before_downloading() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let old_client = b"previous client";
    std::fs::write(dir.join("tilr"), old_client).unwrap();
    let root = dir.join("cdn");
    let archive = release(STUB);
    publish(&root, &platform, "9.9.9-test", &archive, &archive);
    let file = root
        .join("releases")
        .join(&platform)
        .join("latest.manifest.json");
    let authentic = std::fs::read(&file).unwrap();
    let (url, requests) = cdn::serve_files_with_requests(root);
    for fault in [
        "missing",
        "signature",
        "payload",
        "other key",
        "platform",
        "archive",
        "version",
        "checksum",
        "missing channel",
        "invalid channel",
        "oversize",
    ] {
        let mut envelope: serde_json::Value = serde_json::from_slice(&authentic).unwrap();
        if fault == "missing" {
            std::fs::remove_file(&file).unwrap();
        } else {
            if fault == "signature" {
                envelope["signature"] = "00".repeat(64).into();
            } else if fault == "payload" {
                envelope["payload"] = format!("{} ", envelope["payload"].as_str().unwrap()).into();
            } else if fault == "oversize" {
                envelope["payload"] = "x".repeat(17 << 10).into();
            } else {
                let mut payload: serde_json::Value =
                    serde_json::from_str(envelope["payload"].as_str().unwrap()).unwrap();
                match fault {
                    "platform" => payload["platform"] = "other-platform".into(),
                    "archive" => payload["archive"] = "../untrusted.tar.gz".into(),
                    "version" => payload["version"] = "0.0.0".into(),
                    "checksum" => payload["sha256"] = "invalid".into(),
                    "missing channel" => {
                        payload.as_object_mut().unwrap().remove("channel");
                    }
                    "invalid channel" => payload["channel"] = "latest".into(),
                    _ => (),
                }
                let payload = payload.to_string();
                let seed = if fault == "other key" {
                    [1; 32]
                } else {
                    [0; 32]
                };
                let key = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
                envelope["signature"] = key
                    .sign(payload.as_bytes())
                    .as_ref()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
                    .into();
                envelope["payload"] = payload.into();
            }
            std::fs::write(&file, envelope.to_string()).unwrap();
        }
        requests.lock().unwrap().clear();
        let output = update(&exe, &url);
        assert!(!output.status.success(), "{fault}: {output:?}");
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.ends_with(".tar.gz")),
            "{fault}: requested archive"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), original, "{fault}");
        assert_eq!(
            std::fs::read(dir.join("tilr")).unwrap(),
            old_client,
            "{fault}"
        );
        std::fs::write(&file, &authentic).unwrap();
    }
    let output = update(&exe, &url);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), STUB);
    assert_eq!(std::fs::read(dir.join("tilr")).unwrap(), CLIENT);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_rejects_cross_channel_replay_before_downloading() {
    for (channel, pointer, replayed_pointer) in
        [("stable", "latest", "beta"), ("beta", "beta", "latest")]
    {
        let (dir, exe) = installed();
        let (_, platform) = version_and_platform();
        let original = std::fs::read(&exe).unwrap();
        let old_client = b"previous client";
        std::fs::write(dir.join("tilr"), old_client).unwrap();
        let root = dir.join("cdn");
        let archive = release(STUB);
        let version = "9.9.9-rc.1-test";
        publish(&root, &platform, version, &archive, &archive);
        let checksum: String = Sha256::digest(&archive)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        manifest::write_manifest(&root, &platform, version, replayed_pointer, &checksum);
        let pointers = root.join("releases").join(&platform);
        std::fs::write(pointers.join(pointer), version).unwrap();
        std::fs::copy(
            pointers.join(format!("{replayed_pointer}.manifest.json")),
            pointers.join(format!("{pointer}.manifest.json")),
        )
        .unwrap();
        let (url, requests) = cdn::serve_files_with_requests(root);
        let output = spawn_copied(
            configured_update(&exe, Some(channel))
                .args(["update", "--cdn", &url])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .wait_with_output()
        .unwrap();
        assert!(!output.status.success(), "{channel}: {output:?}");
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.ends_with(".tar.gz")),
            "{channel}: requested archive"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), original, "{channel}");
        assert_eq!(
            std::fs::read(dir.join("tilr")).unwrap(),
            old_client,
            "{channel}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
