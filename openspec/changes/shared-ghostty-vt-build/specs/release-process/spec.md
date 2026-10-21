## ADDED Requirements

### Requirement: Shared Ghostty VT development build

The repository SHALL provide a script that builds the static Ghostty VT library pinned by libghostty-vt-sys, using its ReleaseFast, baseline CPU and no application runtime settings. The script SHALL select the same default macOS SDK as Cargo, and SHALL identify the shared prefix by Ghostty revision, Zig version, platform and SDK. It SHALL publish a libghostty-vt-static pkg-config file only with a completed build and SHALL reuse an already completed prefix without cloning or building again.

#### Scenario: Build and reuse a shared prefix
- **WHEN** a developer runs the shared build script with a supported Zig installation
- **THEN** it prints a prefix containing the static library and share/pkgconfig/libghostty-vt-static.pc
- **AND** a subsequent invocation with the same inputs reuses that prefix without cloning Ghostty or invoking zig build

#### Scenario: Toolchain identity changes
- **WHEN** the Ghostty revision, Zig version, platform or selected SDK changes
- **THEN** the script selects a distinct prefix

#### Scenario: Failed build is retried explicitly
- **WHEN** a shared build fails before completing
- **THEN** no completed prefix is published and a later invocation can build it again

### Requirement: Cargo discovers the shared Ghostty VT library

The tilr-screen dependency graph SHALL enable libghostty-vt-sys pkg-config discovery. A matching library supplied through PKG_CONFIG_PATH SHALL replace the per-target source build; when no package is discoverable, the existing source build SHALL remain available. Developer documentation SHALL explain setting PKG_CONFIG_PATH and rebuilding after a dependency or toolchain upgrade.

#### Scenario: Clean Cargo build with shared library
- **WHEN** a clean cargo build --tests runs with PKG_CONFIG_PATH pointing to the shared package
- **THEN** libghostty-vt-sys links that static library without cloning Ghostty or invoking zig build

#### Scenario: Cargo source fallback
- **WHEN** cargo build --tests runs without a discoverable shared package
- **THEN** libghostty-vt-sys builds the pinned Ghostty source as before
