//! Locate supervisors of this user's installation and request reload explicitly.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
use anyhow::ensure;
use anyhow::{Context, Result};
use clap::Parser;

/// Discover before signalling: check children have the same command line as
/// their supervisor, but must never receive a reload signal themselves.
pub fn request(exe: &Path) -> Result<()> {
    let exe = std::fs::canonicalize(exe).context("resolve the installation path for reload")?;
    let candidates: Vec<(i32, i32)> = processes()?
        .into_iter()
        .filter(|(pid, _)| is_supervisor(*pid, &exe))
        .collect();
    for &(pid, parent) in &candidates {
        if candidates.iter().any(|(candidate, _)| *candidate == parent) {
            continue;
        }
        // Recheck after discovery; exited processes are harmless.
        if !is_supervisor(pid, &exe) {
            continue;
        }
        // SAFETY: kill has no memory-safety preconditions.
        if unsafe { libc::kill(pid, libc::SIGUSR1) } != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::ESRCH) {
                return Err(err).with_context(|| format!("request reload from {pid}"));
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn processes() -> Result<Vec<(i32, i32)>> {
    use std::os::unix::fs::MetadataExt;

    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let entries = std::fs::read_dir("/proc").context("list processes for reload")?;
    Ok(entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: i32 = entry.file_name().to_str()?.parse().ok()?;
            if pid <= 0 || entry.metadata().ok()?.uid() != uid {
                return None;
            }
            // A process can exit or become unreadable during discovery.
            let stat = std::fs::read(entry.path().join("stat")).ok()?;
            // comm is parenthesized and may itself contain spaces or ')'.
            let fields = stat.get(stat.iter().rposition(|byte| *byte == b')')? + 1..)?;
            let parent = std::str::from_utf8(fields)
                .ok()?
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()?;
            Some((pid, parent))
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn processes() -> Result<Vec<(i32, i32)>> {
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() }.to_string();
    let output = Command::new("ps")
        .args(["-u", &uid, "-o", "pid=,ppid="])
        .output()
        .context("list this user's processes for reload")?;
    ensure!(output.status.success(), "cannot list processes for reload");
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            Some((words.next()?.parse().ok()?, words.next()?.parse().ok()?))
        })
        .collect())
}

fn is_supervisor(pid: i32, exe: &Path) -> bool {
    let Some((image, args)) = process(pid) else {
        return false;
    };
    let image = std::fs::canonicalize(&image).unwrap_or(image);
    image == exe && crate::Cli::try_parse_from(args).is_ok_and(|cli| cli.command.is_none())
}

#[cfg(target_os = "linux")]
fn process(pid: i32) -> Option<(PathBuf, Vec<OsString>)> {
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let image = std::fs::read_link(dir.join("exe")).ok()?;
    // Linux appends this suffix when an atomic installation unlinks the
    // running image. The installation path remains the reload destination.
    use std::os::unix::ffi::OsStrExt;
    let bytes = image.as_os_str().as_bytes();
    let image = PathBuf::from(OsString::from_vec(
        bytes.strip_suffix(b" (deleted)").unwrap_or(bytes).to_vec(),
    ));
    let bytes = std::fs::read(dir.join("cmdline")).ok()?;
    let args = bytes
        .strip_suffix(&[0])?
        .split(|byte| *byte == 0)
        .map(|arg| OsString::from_vec(arg.to_vec()))
        .collect();
    Some((image, args))
}

#[cfg(target_os = "macos")]
fn process(pid: i32) -> Option<(PathBuf, Vec<OsString>)> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut bytes = vec![0u8; 1024 * 1024];
    let mut len = bytes.len();
    // SAFETY: mib and the output buffer are valid for their supplied lengths.
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as _,
            bytes.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    bytes.truncate(len);
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    let rest = bytes.get(4..)?;
    let end = rest.iter().position(|byte| *byte == 0)?;
    let image = PathBuf::from(OsString::from_vec(rest[..end].to_vec()));
    let rest = &rest[end..];
    let start = rest.iter().position(|byte| *byte != 0)?;
    let args = rest[start..]
        .split(|byte| *byte == 0)
        .take(usize::try_from(argc).ok()?)
        .map(|arg| OsString::from_vec(arg.to_vec()))
        .collect();
    Some((image, args))
}
