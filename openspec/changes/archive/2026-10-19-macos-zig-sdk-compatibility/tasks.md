## Implementation

- [x] Record the failing and successful clean macOS builds, including versions and selected SDK.
- [x] Add the tracked Cargo SDK configuration and macOS SDK preflight to `scripts/install-tools.sh`.
- [x] Select Xcode's macOS 15.4 SDK in CI and release; run the regression test in CI.
- [x] Update README and AGENTS with dated compatibility, recheck, and removal guidance.
- [x] Add and run a deterministic regression test for the SDK configuration and preflight.
- [x] Build in a fresh worktree with an empty target and run the Linux build command.
- [x] Run OpenSpec validation, record the root cause in TILR-164, commit with task trailer, and archive the change.
