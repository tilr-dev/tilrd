# Publish macOS releases on the CDN

## Why
The native updater reads the same per-platform release layout on macOS as on Linux, but macOS builds are currently attached only to GitHub releases. Updates need signed, notarized macOS artifacts on the CDN.

## What Changes
- Build both macOS architectures through the release script, sign both executables with Developer ID, and require accepted notarization before packaging.
- Publish macOS archives and checksums before their per-platform latest pointers, after GitHub assets are attached.
- Verify publication and native updates using a local CDN.

## Impact
- Release scripts, GitHub release workflow, release-process specification, release and update integration tests.
- Apple signing and notarization credentials in the cdn environment; first installation remains outside this change.
