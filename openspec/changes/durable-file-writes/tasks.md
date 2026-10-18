# Tasks

## 1. Sync in writes and update

- [ ] 1.1 fsync file and directory in FsWrite; verify by an strace-based itests check (`fsync` before `rename` and on the directory) in UC-046
- [ ] 1.2 fsync in update; verify the same way in UC-060
- [ ] 1.3 README "Files" and "Update" say a success answer means the data is on disk; verify by review against this proposal

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
