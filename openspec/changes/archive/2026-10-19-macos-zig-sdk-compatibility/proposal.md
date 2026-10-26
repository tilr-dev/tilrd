# macOS Zig SDK compatibility

## Why

As of 2026-10-19, a fresh `cargo build --workspace --features tilrd/failpoints` on Dan's Mac uses Zig 0.15.2 and the default macOS 27.0 SDK. `libghostty-vt-sys` 0.2.2 fails while Zig compiles libc++ because `INFINITY` is undeclared. The same clean build succeeds with the installed Command Line Tools macOS 15.4 SDK. An untracked `.cargo/config.toml` in the main clone provided that selection, but fresh worktrees do not inherit it.

Recheck with a fresh `CARGO_TARGET_DIR` and the build command above, first with the default SDK (`xcrun --show-sdk-version`) and then with `DEVELOPER_DIR=/Library/Developer/CommandLineTools SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk`. Once a pinned Zig release builds with the current SDK, remove the SDK pin, its preflight, and the dated workaround documentation.

## Decision

Keep Zig 0.15.2, which the pinned Ghostty source accepts. Track the Cargo SDK selection in the repository and check for the required Command Line Tools SDK before installing Zig on macOS. Keep the Linux build working.

## What Changes

- Track the Cargo configuration that selects Command Line Tools and macOS 15.4 SDK for a direct build from any worktree.
- Make `scripts/install-tools.sh` check that SDK on macOS before downloading Zig, with actionable failure text.
- Select Xcode 16.3's macOS 15.4 SDK explicitly on the macOS CI and release runners, which may not have it under Command Line Tools.
- Replace the local-workaround instructions with dated setup, recheck and removal instructions.
- Add a deterministic regression check for the tracked configuration and installer preflight.

## Capabilities

- `release-process`: reliable clean macOS build with pinned Zig.

## Non-goals

- Upgrading Zig or removing the SDK compatibility pin; TILR-165 covers that work.
- Installing Apple's SDK itself.

## Impact

New macOS builds require Command Line Tools containing MacOSX15.4.sdk. The repository-selected SDK also reaches Cargo build scripts; the Linux container build must be verified.

Implemented by: `fb05cf6`.
