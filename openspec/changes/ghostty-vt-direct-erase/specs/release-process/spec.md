## MODIFIED Requirements

### Requirement: Shared Ghostty VT development build

The repository SHALL provide a script that builds the static Ghostty VT library pinned by libghostty-vt-sys, using its ReleaseFast, baseline CPU and no application runtime settings. Both the shared script and the vendored Cargo source fallback SHALL apply the same checked-in patch to the pinned source. The script SHALL select the same default macOS SDK as Cargo, and SHALL identify the shared prefix by Ghostty revision, patch content identity, Zig version, platform and SDK. It SHALL publish a libghostty-vt-static pkg-config file only with a completed build and SHALL reuse an already completed prefix without cloning or building again.

#### Scenario: Build and reuse a shared prefix
- **WHEN** a developer runs the shared build script with a supported Zig installation
- **THEN** it prints a prefix containing the static library and share/pkgconfig/libghostty-vt-static.pc
- **AND** a subsequent invocation with the same inputs reuses that prefix without cloning Ghostty or invoking zig build

#### Scenario: Toolchain identity changes
- **WHEN** the Ghostty revision, patch content, Zig version, platform or selected SDK changes
- **THEN** the script selects a distinct prefix

#### Scenario: Failed build is retried explicitly
- **WHEN** a shared build fails before completing
- **THEN** no completed prefix is published and a later invocation can build it again

### Requirement: Cargo discovers the shared Ghostty VT library

The tilr-screen dependency graph SHALL enable libghostty-vt-sys pkg-config discovery. A matching patched library supplied through PKG_CONFIG_PATH SHALL replace the per-target source build; when no package is discoverable, the existing source build SHALL remain available. Discovery SHALL reject packages without the required patch identity. Developer documentation SHALL explain setting PKG_CONFIG_PATH and rebuilding after a dependency or toolchain upgrade.

#### Scenario: Clean Cargo build with shared library
- **WHEN** a clean cargo build --tests runs with PKG_CONFIG_PATH pointing to the shared package
- **THEN** libghostty-vt-sys links that static library without cloning Ghostty or invoking zig build

#### Scenario: Cargo source fallback
- **WHEN** cargo build --tests runs without a discoverable shared package
- **THEN** libghostty-vt-sys builds the pinned Ghostty source with the checked-in patch

## ADDED Requirements

### Requirement: Direct terminal screen erase

**Verified by:** `direct_erase::erase_preserves_split_csi_and_rendition`, `direct_erase::erase_preserves_split_osc`, `direct_erase::erase_preserves_split_dcs`, `direct_erase::erase_homes_absolutely_and_preserves_origin_margins_and_modes`, `direct_erase::erase_applies_history_policy_on_primary_and_alternate_screens`.
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
