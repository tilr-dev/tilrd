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
