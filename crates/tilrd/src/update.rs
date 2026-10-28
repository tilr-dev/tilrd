//! `tilrd update`: replace this executable and the client next to it with
//! the latest release.
//!
//! Releases are files under one base URL:
//!
//! ```text
//! releases/<platform>/beta                                   the candidate for the platform
//! releases/<platform>/latest                                 the version ready for the platform
//! releases/<version>/tilr-<version>-<platform>.tar.gz        tilrd and tilr
//! releases/<platform>/latest.manifest.json                   signed stable metadata
//! releases/<platform>/beta.manifest.json                     signed candidate metadata
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

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

impl Channel {
    fn pointer(self) -> &'static str {
        match self {
            Self::Stable => "latest",
            Self::Beta => "beta",
        }
    }
}

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
fn archive_name(version: &str, platform: &str) -> String {
    format!("tilr-{version}-{platform}.tar.gz")
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: String,
    platform: String,
    archive: String,
    sha256: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: String,
    signature: String,
}

fn unhex(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2) && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid hexadecimal metadata"
    );
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).context("decode hexadecimal metadata"))
        .collect()
}

fn latest_version(cdn: &str, platform: &str, channel: Channel) -> Result<Manifest> {
    let cdn = cdn.trim_end_matches('/');
    let pointer = channel.pointer();
    let latest = fetch_text(&format!("{cdn}/releases/{platform}/{pointer}"))
        .with_context(|| format!("find the latest release for {platform} at {cdn}"))?;
    let latest = latest.trim();
    // It becomes part of a URL.
    ensure!(
        !latest.is_empty()
            && latest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_')),
        "{cdn} names a release that is not a version: {latest:?}"
    );
    let url = format!("{cdn}/releases/{platform}/{pointer}.manifest.json");
    let bytes = match ureq::get(&url).call() {
        Ok(mut response) => response
            .body_mut()
            .with_config()
            .limit(16 << 10)
            .read_to_vec()
            .context("fetch signed manifest")?,
        Err(ureq::Error::StatusCode(404)) => {
            bail!("{cdn} has no release {latest} for {platform}: signed manifest missing")
        }
        Err(err) => return Err(err).context("fetch signed manifest"),
    };
    let envelope: Envelope = serde_json::from_slice(&bytes).context("invalid signed manifest")?;
    let key = unhex(env!("TILR_UPDATE_PUBLIC_KEY"))?;
    let signature = unhex(&envelope.signature)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(envelope.payload.as_bytes(), &signature)
        .map_err(|_| {
            anyhow::anyhow!("update manifest signature is invalid, nothing was installed")
        })?;
    let manifest: Manifest =
        serde_json::from_str(&envelope.payload).context("invalid manifest payload")?;
    ensure!(
        manifest.version == latest
            && manifest.platform == platform
            && manifest.archive == archive_name(latest, platform)
            && manifest.sha256.len() == 64
            && manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "update manifest does not match the selected release, nothing was installed"
    );
    Ok(manifest)
}

fn download(cdn: &str, manifest: &Manifest) -> Result<Vec<u8>> {
    let url = format!("{cdn}/releases/{}/{}", manifest.version, manifest.archive);
    let expected = manifest.sha256.to_ascii_lowercase();

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

    Ok(archive)
}

/// Fetch or validate a release without running its foreign-platform programs.
pub fn fetch_release(cdn: &str, platform: &str, out: &Path, local: Option<&Path>) -> Result<()> {
    ensure!(
        [
            "linux-x86_64",
            "linux-aarch64",
            "macos-aarch64",
            "macos-x86_64"
        ]
        .contains(&platform),
        "unsupported release platform: {platform}"
    );
    let (name, archive) = if let Some(path) = local {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("archive file name")?;
        ensure!(
            name.starts_with("tilr-") && name.ends_with(&format!("-{platform}.tar.gz")),
            "archive {name} is not a release for {platform}"
        );
        ensure!(
            fs::metadata(path)?.len() <= MAX_ARCHIVE,
            "archive is too large"
        );
        let archive = fs::read(path)?;
        let mut checksum_path = path.as_os_str().to_owned();
        checksum_path.push(".sha256");
        let checksum =
            fs::read_to_string(PathBuf::from(checksum_path)).context("read archive checksum")?;
        ensure!(
            checksum
                .split_whitespace()
                .next()
                .is_some_and(|sum| sum.eq_ignore_ascii_case(&sha256(&archive))),
            "checksum mismatch, nothing was installed"
        );
        (name.to_owned(), archive)
    } else {
        let cdn = cdn.trim_end_matches('/');
        let latest = latest_version(cdn, platform, Channel::Stable).with_context(|| {
            format!("no release of tilr for {platform} on {cdn}; give one with --archive")
        })?;
        let archive = download(cdn, &latest)?;
        (latest.archive, archive)
    };
    fs::create_dir_all(out)?;
    let staging = out.join(format!(".tilr-fetch.{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&staging)?;
    let staged = Staged(staging);
    unpack(&archive, &staged.0).context("validate the release archive")?;
    let target = out.join(name);
    let checked = staged.0.join("archive.tar.gz");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&checked)?
        .write_all(&archive)?;
    // Publish only a complete checked file, without replacing an existing archive.
    fs::hard_link(&checked, &target)?;
    println!("{}", target.display());
    Ok(())
}

pub fn run(cdn: &str, channel: Channel, exe: &Path) -> Result<()> {
    let cdn = cdn.trim_end_matches('/');
    let latest = latest_version(cdn, PLATFORM, channel)?;
    if latest.version == VERSION {
        println!("tilrd {VERSION} ({PLATFORM}) is the latest release");
        return Ok(());
    }

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

    println!("downloading tilr {} ({PLATFORM})", latest.version);
    let archive = download(cdn, &latest)?;
    let latest = latest.version;

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
    crate::reload_request::request(exe)
        .context("release installed, but could not request reload")?;
    println!("a running server reloads itself and keeps its shells");
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
