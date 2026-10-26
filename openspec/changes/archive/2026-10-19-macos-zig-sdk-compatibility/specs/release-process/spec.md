## ADDED Requirements

### Requirement: Pinned Zig uses a compatible macOS SDK in clean worktrees
While the build tools use Zig 0.15.2, the repository SHALL select the Command Line Tools macOS 15.4 SDK by default for Cargo builds on macOS, including clean worktrees without local Cargo configuration. A caller MAY select another installed macOS 15.4 SDK with `DEVELOPER_DIR` and `SDKROOT`. The Linux build image SHALL continue to build without a macOS SDK.

#### Scenario: Direct clean macOS build
- **WHEN** a fresh worktree on a Mac with the Command Line Tools macOS 15.4 SDK runs `cargo build --workspace --features tilrd/failpoints` with an empty target directory and the default SDK is macOS 27.0
- **THEN** `libghostty-vt-sys` 0.2.2 builds with Zig 0.15.2 using the selected macOS 15.4 SDK and the workspace build succeeds

#### Scenario: CI uses Xcode's macOS 15.4 SDK
- **WHEN** a macOS CI or release runner has macOS 15.4 SDK in Xcode 16.3 rather than Command Line Tools
- **THEN** the workflow selects that Xcode and SDK for the installer and Cargo build

#### Scenario: Linux build image
- **WHEN** `scripts/linux.sh cargo build --workspace --features tilrd/failpoints --locked` runs
- **THEN** the build succeeds without a macOS SDK

**Verified by:** `scripts/test-macos-sdk.sh`, a fresh macOS workspace build, and a fresh Linux workspace build via `scripts/linux.sh`.

### Requirement: macOS Zig installer checks its SDK
`scripts/install-tools.sh DIR zig` on macOS SHALL verify the selected macOS 15.4 SDK before downloading Zig and SHALL fail with setup guidance when it is absent.

#### Scenario: Missing compatible SDK during tool installation
- **WHEN** `scripts/install-tools.sh DIR zig` runs on macOS without the selected macOS 15.4 SDK
- **THEN** it fails before downloading Zig and explains how to install or select the required SDK

**Verified by:** `scripts/test-macos-sdk.sh`.
