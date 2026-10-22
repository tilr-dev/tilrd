//! `tilrd update`: replace this executable and the client next to it with
//! the latest release.
//!
//! Releases are files under one base URL:
//!
//! ```text
//! releases/<platform>/latest                                 the version ready for the platform
//! releases/<version>/tilr-<version>-<platform>.tar.gz        tilrd and tilr
//! releases/<version>/tilr-<version>-<platform>.tar.gz.sha256
//! ```
//!
//! The archive is unpacked into a directory next to the executable, both
//! programs are checked to run, and then renamed over the installed ones:
//! `tilr` first, `tilrd` (this executable) last. A server running from it
//! is explicitly asked to reload, keeping its shells.
//!
//! Until those renames nothing has changed: an update that fails, or is
//! killed, leaves the installation as it was. If `tilrd` cannot be put in
//! place after `tilr` was, the old `tilr` is put back. One update runs at a
//! time.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use crate::{PLATFORM, VERSION};

pub const DEFAULT_CDN: &str = "https://cdn.tilr.dev";

/// No release comes near this; a larger answer is not one.
const MAX_ARCHIVE: u64 = 256 << 20;
/// Nor a program in it.
const MAX_PROGRAM: u64 = 512 << 20;

/// The programs of a release, in the order they are put in place: the
/// server last, before explicitly requesting reload.
const PROGRAMS: [&str; 2] = ["tilr", "tilrd"];

/// What the directory an update unpacks into is called, before the pid.
const STAGING: &str = ".tilr-update.";

/// The file name of release `version` for this platform.
fn archive_name(version: &str) -> String {
    format!("tilr-{version}-{PLATFORM}.tar.gz")
}

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

    let url = format!("{cdn}/releases/{latest}/{}", archive_name(latest));
    let checksum = match ureq::get(&format!("{url}.sha256")).call() {
        Ok(mut response) => response
            .body_mut()
            .read_to_string()
            .context("fetch the checksum")?,
        Err(ureq::Error::StatusCode(404)) => {
            bail!("{cdn} has no release {latest} for {PLATFORM}")
        }
        Err(err) => return Err(err).context("fetch the checksum"),
    };
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
    remove_leftovers(dir);

    println!("downloading tilr {latest} ({PLATFORM})");
    let archive = ureq::get(&url)
        .call()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(MAX_ARCHIVE)
                .read_to_vec()
        })
        .with_context(|| format!("download {url}"))?;
    ensure!(
        sha256(&archive) == expected,
        "the download does not match its checksum, nothing was installed"
    );

    // Next to the executable, so that the renames stay on one filesystem.
    let staged = Staged(dir.join(format!("{STAGING}{}", std::process::id())));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&staged.0)
        .with_context(|| format!("write {}", staged.0.display()))?;
    unpack(&archive, &staged.0)
        .with_context(|| format!("unpack release {latest}, nothing was installed"))?;

    // Where each program goes: the server where this one is, under whatever
    // name it was installed as; the client next to it.
    let targets = [dir.join("tilr"), exe.to_owned()];
    let same = PROGRAMS.iter().zip(&targets).all(|(name, target)| {
        fs::read(target)
            .is_ok_and(|installed| fs::read(staged.0.join(name)).ok() == Some(installed))
    });
    if same {
        println!("tilr {latest} ({PLATFORM}) is already installed");
        return Ok(());
    }
    for name in PROGRAMS {
        let runs = Command::new(staged.0.join(name))
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !runs {
            bail!("{name} {latest} does not run on this system, nothing was installed");
        }
    }
    crate::fail::point("update-before-rename")?;

    // The client first, its old file kept until the server is in place too.
    let [client, server] = &targets;
    let old_client = staged.0.join("tilr.old");
    let had_client = fs::hard_link(client, &old_client).is_ok();
    fs::rename(staged.0.join("tilr"), client)
        .with_context(|| format!("replace {}", client.display()))?;
    let swapped = crate::fail::point("update-between-renames").and_then(|()| {
        fs::rename(staged.0.join("tilrd"), server)
            .with_context(|| format!("replace {}", server.display()))
    });
    if let Err(err) = swapped {
        let _ = if had_client {
            fs::rename(&old_client, client)
        } else {
            fs::remove_file(client)
        };
        return Err(err.context("nothing was installed"));
    }
    // The new names on disk before success is reported.
    crate::durable::sync_dir(dir).with_context(|| format!("sync {}", dir.display()))?;

    // A server may end this update exec as soon as it receives the request.
    // Finish cleanup and report the installation before asking it to reload.
    drop(staged);
    drop(lock);
    println!("updated {VERSION} -> {latest}: {}", dir.display());
    println!("a running server reloads itself and keeps its shells");
    crate::reload_request::request(exe)
        .context("release installed, but could not request reload")?;
    Ok(())
}

/// Write `tilr` and `tilrd` of a release archive into `dir`, executable.
/// Nothing else in it is written; either missing is an error.
fn unpack(archive: &[u8], dir: &Path) -> Result<()> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut found = [false; PROGRAMS.len()];
    for entry in tar.entries().context("read the archive")? {
        let mut entry = entry.context("read the archive")?;
        let path = entry.path().context("read the archive")?.into_owned();
        let name = path.strip_prefix(".").unwrap_or(&path);
        let Some(index) = PROGRAMS
            .iter()
            .position(|program| name == Path::new(program))
        else {
            continue;
        };
        let program = PROGRAMS[index];
        ensure!(
            entry.header().entry_type().is_file(),
            "{program} in the archive is not a file"
        );
        ensure!(
            entry.size() <= MAX_PROGRAM,
            "{program} in the archive is too large"
        );
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).context("read the archive")?;
        let target = dir.join(program);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(&target)
            .and_then(|mut file| {
                file.write_all(&bytes)?;
                // On disk before it is renamed into place.
                file.sync_all()
            })
            .with_context(|| format!("write {}", target.display()))?;
        found[index] = true;
    }
    for (program, found) in PROGRAMS.iter().zip(found) {
        ensure!(found, "the archive has no {program}");
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Remove what updates left that were killed before their renames. Called
/// with the lock held, so none of it is in use.
fn remove_leftovers(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(STAGING) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn fetch_text(url: &str) -> Result<String> {
    Ok(ureq::get(url).call()?.body_mut().read_to_string()?)
}

/// The unpacked release until it is renamed into place; removed with what is
/// left in it either way.
struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
