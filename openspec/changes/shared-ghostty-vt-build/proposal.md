## Why

Each clean Cargo target currently clones and builds Ghostty. A shared native library lets task worktrees reuse that work while preserving the source-build fallback.

## What Changes

- Enable the sys crate's pkg-config feature directly in tilr-screen.
- Add an idempotent shared-library build script pinned to the sys crate's Ghostty revision and build flags, including the macOS SDK selection.
- Document the cache identity, pkg-config setup and upgrade procedure.

## Impact

- Affected spec: release-process (ADDED requirements).
- Affected code: tilr-screen dependencies, build scripts and developer documentation.
