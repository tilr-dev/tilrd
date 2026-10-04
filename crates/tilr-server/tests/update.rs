//! Use cases: `tilr-server update` installs the latest release.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;

use common::*;
use sha2::{Digest, Sha256};

/// A CDN in a directory: serves the files under `root` on a loopback port.
/// Returns the base URL.
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
            let _ = match std::fs::read(root.join(path.trim_start_matches('/'))) {
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

fn update(exe: &Path, cdn: &str) -> std::process::Output {
    Command::new(exe)
        .args(["update", "--cdn", cdn])
        .output()
        .unwrap()
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
    // The "new version" is this build with a byte more: another file, which
    // runs the same.
    let mut release = std::fs::read(&exe).unwrap();
    release.push(0);
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
