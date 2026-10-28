//! Bakes into the executable what build it is: the version (the package
//! version and the commit) and the platform, named as releases name it.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=TILR_UPDATE_PUBLIC_KEY");
    const FIXTURE_KEY: &str = "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29";
    let release = std::env::var("PROFILE").unwrap() == "release";
    let key = std::env::var("TILR_UPDATE_PUBLIC_KEY").unwrap_or_else(|_| {
        assert!(
            !release,
            "release builds require TILR_UPDATE_PUBLIC_KEY (hex Ed25519 public key)"
        );
        FIXTURE_KEY.to_owned()
    });
    assert!(
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid TILR_UPDATE_PUBLIC_KEY"
    );
    assert!(
        !release || !key.eq_ignore_ascii_case(FIXTURE_KEY),
        "release builds must not trust the fixture key"
    );
    println!("cargo:rustc-env=TILR_UPDATE_PUBLIC_KEY={key}");
    println!("cargo:rerun-if-env-changed=TILR_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=TILR_BUILD_VERSION");
    // HEAD changes on checkout (even with the reflog off), logs/HEAD on every
    // commit. Asked of git, so that a worktree, whose `.git` is a file, names
    // the real files; a watched file that does not exist would make cargo
    // rerun this script on every build, so without git nothing is watched.
    for name in ["HEAD", "logs/HEAD"] {
        if let Some(path) = git_path(name) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let package = std::env::var("TILR_BUILD_VERSION")
        .unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
    // A build that cannot ask git (a container that does not own the source
    // tree) is told the commit; one that knows no commit says so.
    let commit = std::env::var("TILR_BUILD_COMMIT")
        .ok()
        .filter(|commit| !commit.is_empty())
        .or_else(git_commit)
        .unwrap_or_else(|| "dev".into());
    println!("cargo:rustc-env=TILR_VERSION={package}-{commit}");

    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    println!("cargo:rustc-env=TILR_PLATFORM={os}-{arch}");
}

fn git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !commit.is_empty()).then_some(commit)
}

/// The file `name` inside the real git directory (a worktree's too), if git
/// can name it and it exists.
fn git_path(name: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    // git may name it relative to the current directory, the crate's.
    std::fs::canonicalize(path).ok()
}
