//! `tilrd update`: replace this executable with the latest release.
//!
//! Releases are files under one base URL:
//!
//! ```text
//! releases/<platform>/latest                         the version ready for the platform
//! releases/<version>/tilrd-<platform>
//! releases/<version>/tilrd-<platform>.sha256
//! ```
//!
//! Releases before the rename to `tilrd` are named `tilr-server-<platform>`,
//! and those after it are published under that name too, for the servers
//! still called `tilr-server`, which ask for it: they update to `tilrd` in
//! place, under the name they were installed as.
//!
//! The download is renamed over this executable. A server running from it
//! notices and reloads, keeping its shells; nothing is restarted here.
//!
//! Until that rename nothing has changed: an update that fails, or is killed,
//! leaves the executable as it was. One update runs at a time.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use crate::{PLATFORM, VERSION};

pub const DEFAULT_CDN: &str = "https://cdn.tilr.dev";

/// No release comes near this; a larger answer is not one.
const MAX_BINARY: u64 = 256 << 20;

pub fn run(cdn: &str, exe: &Path) -> Result<()> {
    ensure!(
        !exe.with_file_name("app-managed.json").exists(),
        "this server is managed by Tilr.app; use Tilr > Check for Server Updates for the independent signed server channel"
    );
    let cdn = cdn.trim_end_matches('/');
    let latest = fetch_text(&format!("{cdn}/releases/{PLATFORM}/latest"))
        .with_context(|| format!("find the latest release for {PLATFORM} at {cdn}"))?;
    let latest = latest.trim();
    // It becomes part of a URL.
    ensure!(
        !latest.is_empty()
            && latest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_')),
        "{cdn} names a release that is not a version: {latest:?}"
    );
    if latest == VERSION {
        println!("tilrd {VERSION} ({PLATFORM}) is the latest release");
        return Ok(());
    }

    let (url, checksum) = find_release(cdn, latest)?;
    let expected = checksum
        .split_whitespace()
        .next()
        .context("the checksum file is empty")?
        .to_ascii_lowercase();

    // One update at a time: a second one waits here, then finds the release
    // installed. The lock is on the directory and goes with this process.
    let dir = exe.parent().context("executable path")?;
    let lock = File::open(dir).with_context(|| format!("open {}", dir.display()))?;
    // SAFETY: flock has no memory-safety preconditions.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == -1 {
        return Err(std::io::Error::last_os_error())
            .context("lock the directory of the executable");
    }
    let mut staged_name = exe.file_name().context("executable path")?.to_owned();
    staged_name.push(".new.");
    remove_leftovers(dir, &staged_name.to_string_lossy());
    if fs::read(exe).is_ok_and(|installed| sha256(&installed) == expected) {
        println!("tilrd {latest} ({PLATFORM}) is already installed");
        return Ok(());
    }

    println!("downloading tilrd {latest} ({PLATFORM})");
    let binary = ureq::get(&url)
        .call()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(MAX_BINARY)
                .read_to_vec()
        })
        .with_context(|| format!("download {url}"))?;
    ensure!(
        sha256(&binary) == expected,
        "the download does not match its checksum, nothing was installed"
    );

    // Next to the executable, so that the rename stays on one filesystem.
    staged_name.push(std::process::id().to_string());
    let staged = Staged(exe.with_file_name(staged_name));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(&staged.0)
        .and_then(|mut file| file.write_all(&binary))
        .with_context(|| format!("write {}", staged.0.display()))?;
    let runs = Command::new(&staged.0)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !runs {
        bail!("tilrd {latest} does not run on this system, nothing was installed");
    }
    crate::fail::point("update-before-rename")?;
    fs::rename(&staged.0, exe).with_context(|| format!("replace {}", exe.display()))?;

    println!("updated {VERSION} -> {latest}: {}", exe.display());
    println!("a running server reloads itself and keeps its shells");
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Remove downloads left by updates that were killed before their rename.
/// Called with the lock held, so none of them is in use.
fn remove_leftovers(dir: &Path, prefix: &str) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// What a release is called, newest name first.
const NAMES: [&str; 2] = ["tilrd", "tilr-server"];

/// The URL of release `version` and its checksum file, under whichever name
/// it was published: one from before the rename has only the old one.
fn find_release(cdn: &str, version: &str) -> Result<(String, String)> {
    for name in NAMES {
        let url = format!("{cdn}/releases/{version}/{name}-{PLATFORM}");
        match ureq::get(&format!("{url}.sha256")).call() {
            Ok(mut response) => {
                let checksum = response
                    .body_mut()
                    .read_to_string()
                    .context("fetch the checksum")?;
                return Ok((url, checksum));
            }
            Err(ureq::Error::StatusCode(404)) => {}
            Err(err) => return Err(err).context("fetch the checksum"),
        }
    }
    bail!("{cdn} has no release {version} for {PLATFORM}")
}

fn fetch_text(url: &str) -> Result<String> {
    Ok(ureq::get(url).call()?.body_mut().read_to_string()?)
}

/// The downloaded file until it is renamed into place; removed if it never is.
struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
