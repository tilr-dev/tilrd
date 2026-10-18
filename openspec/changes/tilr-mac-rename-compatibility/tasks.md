# Tasks

## 1. Coordination

- [ ] 1.1 Confirm the merge order with the owner; verify by the merged branches in the listed order
- [ ] 1.2 Publish the first tilrd release with `--min-app-build`; verify the Mac feed refuses older app builds
- [ ] 1.3 The Mac app's release scripts take `tilrd` from the `tilr-vX.Y.Z-<target>.tar.gz` archive (or build `-p tilrd`); verify a bundle built from a GitHub release runs `tilrd --version`
- [ ] 1.4 Delete or recreate the old `tilr-server` feed; verify `cdn.tilr.dev/server/macos/arm64/stable.json` names `tilrd`

## 2. Checks

- [ ] 2.1 Run the checks from AGENTS.md: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the same with `--features tilrd/failpoints`, `cargo test --workspace --features tilrd/failpoints --locked`, `cargo deny check`, `gitleaks git --redact --no-banner --exit-code 1 .`; on macOS also `scripts/linux.sh`; verify all green
