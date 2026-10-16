//! Use cases: `tilr-server update` installs the latest release.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;

use common::*;
use sha2::{Digest, Sha256};

/// `<file>.<extension>`, next to `file`.
fn twin(file: &Path, extension: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(format!(".{extension}"));
    name.into()
}

/// A CDN in a directory: serves the files under `root` on a loopback port.
/// A file that has a twin named `<file>.cut` breaks off halfway. Returns the
/// base URL.
fn serve_files(root: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split(' ').nth(1).unwrap_or("/");
            let file = root.join(path.trim_start_matches('/'));
            let cut = twin(&file, "cut").exists();
            let _ = match std::fs::read(file) {
                Ok(body) if cut => stream.write_all(
                    &[
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len())
                            .as_bytes(),
                        &body[..body.len() / 2],
                    ]
                    .concat(),
                ),
                Ok(body) => stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .and_then(|()| stream.write_all(&body)),
                Err(_) => stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                ),
            };
        }
    });
    format!("http://{addr}")
}

/// Version and platform of the executable under test, as `--version` prints
/// them: `tilr-server 0.1.0-26449dd (linux-aarch64)`.
fn version_and_platform() -> (String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_tilr-server"))
        .arg("--version")
        .output()
        .unwrap();
    let line = String::from_utf8(output.stdout).unwrap();
    let mut words = line.split_whitespace().skip(1);
    let version = words.next().unwrap().to_owned();
    let platform = words.next().unwrap().trim_matches(['(', ')']).to_owned();
    (version, platform)
}

/// Lay out a release of `binary` as `version` for `platform` under `cdn`,
/// and name it the latest one.
fn publish(cdn: &Path, platform: &str, version: &str, binary: &[u8], checksum_of: &[u8]) {
    let dir = cdn.join("releases").join(version);
    std::fs::create_dir_all(&dir).unwrap();
    let name = format!("tilr-server-{platform}");
    std::fs::write(dir.join(&name), binary).unwrap();
    let checksum: String = Sha256::digest(checksum_of)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    std::fs::write(
        dir.join(format!("{name}.sha256")),
        format!("{checksum}  {name}\n"),
    )
    .unwrap();
    let latest = cdn.join("releases").join(platform);
    std::fs::create_dir_all(&latest).unwrap();
    std::fs::write(latest.join("latest"), format!("{version}\n")).unwrap();
}

/// A copy of the server in a directory of its own, and that directory.
fn installed() -> (PathBuf, PathBuf) {
    let dir = Server::scratch_dir();
    let exe = dir.join("tilr-server");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &exe).unwrap();
    (dir, exe)
}

#[test]
fn app_managed_server_never_contacts_the_standalone_update_feed() {
    let (dir, exe) = installed();
    std::fs::write(dir.join("app-managed.json"), "{}").unwrap();
    let result = update(&exe, "http://127.0.0.1:1");
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("managed by Tilr.app"));
    std::fs::remove_dir_all(dir).unwrap();
}

fn update(exe: &Path, cdn: &str) -> std::process::Output {
    update_failing(exe, cdn, "").wait_with_output().unwrap()
}

/// Start an update in which things go wrong where `failpoints` says, see
/// `src/fail.rs`.
fn update_failing(exe: &Path, cdn: &str, failpoints: &str) -> Child {
    spawn_copied(
        Command::new(exe)
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
    let release = new_version(&exe);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &release, &release);
    let cdn = serve_files(cdn);

    let server = Server::start_in(dir.clone(), &exe, &[]);
    let mut client = Client::connect(&server).await;
    let pane_id = client.open_pane().await;
    assert_eq!(client.run(pane_id, "KEPT=yes").await, 0);

    let output = update(&exe, &cdn);
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("-> 9.9.9-test"));
    assert_eq!(std::fs::read(&exe).unwrap(), release);

    // Nobody tells the server: it notices the new file and reloads.
    let mut client = reconnect(&server, client).await;
    assert!(server.logged("executable changed on disk"));
    client.subscribe(pane_id).await;
    assert_eq!(client.run(pane_id, "test \"$KEPT\" = yes").await, 0);
}

#[tokio::test]
async fn update_does_nothing_when_the_latest_release_is_the_one_installed() {
    let (dir, exe) = installed();
    let (version, platform) = version_and_platform();
    let cdn = dir.join("cdn");
    // Only the name of the latest release: nothing to download.
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
    let cdn = dir.join("cdn");
    let url = serve_files(cdn.clone());

    // Not the file the checksum was made of.
    publish(&cdn, &platform, "9.9.9-test", b"tampered", b"released");
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match its checksum"));

    // Intact, but not a program.
    publish(
        &cdn,
        &platform,
        "9.9.9-test",
        b"not a program",
        b"not a program",
    );
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not run on this system"));

    // No release for this platform at all.
    std::fs::remove_dir_all(cdn.join("releases").join(&platform)).unwrap();
    assert!(!update(&exe, &url).status.success());

    // The installed executable is untouched, and nothing is left next to it.
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir), ["cdn", "tilr-server"]);
}

#[tokio::test]
async fn update_installs_nothing_when_the_cdn_misbehaves() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let release = new_version(&exe);
    let cdn = dir.join("cdn");
    let url = serve_files(cdn.clone());
    let latest = cdn.join("releases").join(&platform).join("latest");
    let binary = cdn
        .join("releases/9.9.9-test")
        .join(format!("tilr-server-{platform}"));

    // The download breaks off halfway.
    publish(&cdn, &platform, "9.9.9-test", &release, &release);
    std::fs::write(twin(&binary, "cut"), "").unwrap();
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("download"));

    // The release is named before its files are there.
    std::fs::remove_file(&binary).unwrap();
    assert!(!update(&exe, &url).status.success());
    std::fs::remove_file(twin(&binary, "sha256")).unwrap();
    assert!(!update(&exe, &url).status.success());

    // What answers is not the CDN: a login page, say.
    std::fs::write(&latest, "<html><body>Sign in</body></html>").unwrap();
    let output = update(&exe, &url);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a version"));

    // Nobody answers at all.
    assert!(!update(&exe, "http://127.0.0.1:9").status.success());

    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir), ["cdn", "tilr-server"]);
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
    let exe = dir.join("tilr-server");
    std::fs::copy(env!("CARGO_BIN_EXE_tilr-server"), &exe).unwrap();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let release = new_version(&exe);
    let cdn = scratch.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &release, &release);
    let url = serve_files(cdn);

    std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o555)).unwrap();
    let output = update(&exe, &url);
    std::fs::set_permissions(&dir, PermissionsExt::from_mode(0o755)).unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("write"));
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir), ["tilr-server"]);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn an_update_killed_before_the_swap_changes_nothing_and_the_next_one_tidies_up() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let original = std::fs::read(&exe).unwrap();
    let release = new_version(&exe);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &release, &release);
    let url = serve_files(cdn);

    // Killed with the download on disk, a moment before it is renamed.
    let killed = update_failing(&exe, &url, "update-before-rename=kill")
        .wait_with_output()
        .unwrap();
    assert!(!killed.status.success());
    assert_eq!(std::fs::read(&exe).unwrap(), original);
    assert_eq!(files_in(&dir).len(), 3, "the download is left behind");

    let output = update(&exe, &url);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&exe).unwrap(), release);
    assert_eq!(files_in(&dir), ["cdn", "tilr-server"]);
}

#[cfg(feature = "failpoints")]
#[tokio::test]
async fn two_updates_at_once_install_the_release_once() {
    let (dir, exe) = installed();
    let (_, platform) = version_and_platform();
    let release = new_version(&exe);
    let cdn = dir.join("cdn");
    publish(&cdn, &platform, "9.9.9-test", &release, &release);
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
    assert_eq!(std::fs::read(&exe).unwrap(), release);
    assert_eq!(files_in(&dir), ["cdn", "tilr-server"]);
}
