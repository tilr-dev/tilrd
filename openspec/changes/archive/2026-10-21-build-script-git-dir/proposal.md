# Proposal: Builds are rebuilt only when the commit changes

## Why

The build scripts of `tilr` and `tilrd` watch `../../.git/logs/HEAD` so that a
new commit is baked into the version. In a git worktree `.git` is a file, that
path does not exist, and cargo treats a missing watched file as a change: every
`cargo build` reruns both build scripts and recompiles both crates.

## What Changes

- Ask git for the real git directory's `HEAD` and `logs/HEAD`
  (`git rev-parse --git-path`), so clones and worktrees watch files that exist.
- Watch only the files git names and that exist; when git cannot name them
  (no git, a container build), watch no file. The version logic is unchanged.
- A test of each crate reads its build script's output and checks that every
  watched file exists and that the git files are watched whenever git names them.

## Capabilities

- Add a release-process requirement: builds are rebuilt only when the commit changes.

## Impact

`crates/tilr/build.rs` and `crates/tilrd/build.rs` (kept identical) and a test per
crate. The version an executable carries is unchanged.
