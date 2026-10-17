//! Bakes into the executable what build it is: the version (the package
//! version and the commit) and the platform, named as releases name it.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=TILR_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=TILR_BUILD_VERSION");
    // Written to on every commit and checkout.
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");

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
