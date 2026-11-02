## MODIFIED Requirements

### Requirement: Shared Ghostty VT development build

The shared build script and Cargo fallback SHALL apply the checked-in patch to pinned Ghostty source. The script SHALL use ReleaseFast, baseline CPU, no application runtime and Cargo's default macOS SDK. It SHALL key prefixes by revision, patch content, Zig version, platform and SDK; publish libghostty-vt-static-tilr-erase-v1.pc only after completion; and reuse completed prefixes without cloning or rebuilding.

#### Scenario: Build and reuse a shared prefix
- **WHEN** a developer runs the shared build script with a supported Zig installation
- **THEN** it prints a prefix containing the static library and share/pkgconfig/libghostty-vt-static-tilr-erase-v1.pc
- **AND** a subsequent invocation with the same inputs reuses that prefix without cloning Ghostty or invoking zig build

#### Scenario: Toolchain identity changes
- **WHEN** the Ghostty revision, patch content, Zig version, platform or selected SDK changes
- **THEN** the script selects a distinct prefix

#### Scenario: Failed build is retried explicitly
- **WHEN** a shared build fails before completing
- **THEN** no completed prefix is published and a later invocation can build it again

### Requirement: Cargo discovers the shared Ghostty VT library

The tilr-screen graph SHALL enable patched libghostty-vt-sys pkg-config discovery. A matching patched library in PKG_CONFIG_PATH SHALL replace the source build; without it, the patched source fallback SHALL remain available. Discovery SHALL reject packages without the required patch identity. Developer documentation SHALL explain PKG_CONFIG_PATH and rebuilding after dependency or toolchain upgrades.

#### Scenario: Clean Cargo build with shared library
- **WHEN** a clean cargo build --tests runs with PKG_CONFIG_PATH pointing to the shared package
- **THEN** libghostty-vt-sys links that static library without cloning Ghostty or invoking zig build

#### Scenario: Cargo source fallback
- **WHEN** cargo build --tests runs without a discoverable shared package
- **THEN** libghostty-vt-sys builds the pinned Ghostty source with the checked-in patch

## ADDED Requirements

### Requirement: Direct terminal screen erase

The screen backend SHALL expose an erase operation that clears the active screen, moves the cursor to absolute cell (0,0), and optionally removes its scrollback. It SHALL call the native terminal API without feeding bytes through the program parser or writing to the PTY. It SHALL preserve terminal modes, scrolling margins, current rendition and incomplete program sequences.

#### Scenario: Erase between program sequence chunks
- **WHEN** erase is called between fixed chunks of CSI, OSC or DCS output
- **THEN** the remaining chunk completes the original command and the next printable character has the intended rendition without literal command fragments
- **AND** no PTY bytes are emitted by erase

#### Scenario: Origin mode and scrolling region
- **WHEN** erase is called with origin mode and nondefault scrolling margins
- **THEN** the next printable character starts at absolute (0,0)
- **AND** subsequent program cursor addressing and scrolling still use the original mode and margins

#### Scenario: History policy
- **WHEN** erase is called with history deletion disabled or enabled
- **THEN** existing scrollback is respectively preserved or removed, and all active screen cells are erased
