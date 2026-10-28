//! The build script must watch only files that exist: cargo treats a missing
//! `rerun-if-changed` path as changed, so the crate is rebuilt on every
//! `cargo build`. A git worktree has a `.git` file instead of a directory,
//! which is why the git files are resolved with `git rev-parse --git-path`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The `rerun-if-changed` paths this crate's build script printed; cargo keeps
/// the script's stdout in `<OUT_DIR>/../output`.
fn watched_paths() -> Vec<PathBuf> {
    let output = Path::new(env!("OUT_DIR")).parent().unwrap().join("output");
    let output = std::fs::read_to_string(&output)
        .unwrap_or_else(|err| panic!("read {}: {err}", output.display()));
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    output
        .lines()
        .filter_map(|line| {
            line.strip_prefix("cargo:rerun-if-changed=")
                .or_else(|| line.strip_prefix("cargo::rerun-if-changed="))
        })
        .map(|path| manifest_dir.join(path))
        .collect()
}

/// The file `name` inside the real git directory, as the build script resolves it.
fn git_path(name: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", name])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
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
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).ok()
}

#[test]
fn every_watched_file_exists_and_the_git_head_is_watched() {
    let watched = watched_paths();
    for path in &watched {
        assert!(
            path.exists(),
            "the build script watches {}, which does not exist, so cargo rebuilds the crate on every build",
            path.display()
        );
    }
    match git_path("HEAD") {
        Some(head) => {
            assert!(
                watched.contains(&head),
                "{} is not watched: {watched:?}",
                head.display()
            );
            if let Some(log) = git_path("logs/HEAD") {
                assert!(
                    watched.contains(&log),
                    "{} is not watched: {watched:?}",
                    log.display()
                );
            }
        }
        None => assert!(watched.is_empty(), "no git, yet watched: {watched:?}"),
    }
}

#[test]
fn release_builds_cannot_trust_the_debug_fixture_key() {
    let dir = std::env::temp_dir().join(format!("tilrd-key-build-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let script = dir.join("build-script");
    let output = Command::new("rustc")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .arg("-o")
        .arg(&script)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let fixture = "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29";
    for (profile, key, allowed) in [
        ("release", None, false),
        ("release", Some(fixture.to_owned()), false),
        ("release", Some(fixture.to_uppercase()), false),
        ("release", Some("malformed".into()), false),
        ("release", Some("11".repeat(32)), true),
        ("debug", None, true),
    ] {
        let mut cmd = Command::new(&script);
        cmd.env("PROFILE", profile)
            .env_remove("TILR_UPDATE_PUBLIC_KEY")
            .env("CARGO_PKG_VERSION", "0.0.0")
            .env("CARGO_CFG_TARGET_OS", "linux")
            .env("CARGO_CFG_TARGET_ARCH", "x86_64");
        if let Some(key) = key {
            cmd.env("TILR_UPDATE_PUBLIC_KEY", key);
        }
        let output = cmd.output().unwrap();
        assert_eq!(output.status.success(), allowed, "{profile}: {output:?}");
        if allowed {
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("cargo:rustc-env=TILR_UPDATE_PUBLIC_KEY=")
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
