# Raise the minimum supported Rust version

## Why

The project distributes ready-built binaries and does not need an older Rust baseline. Raising the minimum version allows current atomic APIs without deprecation exceptions.

## What Changes

- Raise the workspace MSRV and CI gate from Rust 1.90 to Rust 1.99.
- Keep build documentation and release gate assertions aligned with the new baseline.
- Use `AtomicU64::try_update` for the frame measurement clock.

## Capabilities

### Modified Capabilities
- `release-process`: require the MSRV gate to build with Rust 1.99.

## Impact

Workspace manifest, CI workflow, contributor and build documentation, release gate tests, and frame measurement clock implementation.
