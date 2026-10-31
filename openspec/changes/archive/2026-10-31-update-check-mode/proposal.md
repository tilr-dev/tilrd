# Check the configured release without installing it

## Why
Inspecting the release on the configured channel currently requires reading CDN metadata manually.

## What Changes
- Add `tilrd update --check`, reusing authenticated release discovery.
- Print installed and available versions and the configured channel without downloading an archive or changing files.
- Use exit 0 for the same version, 2 for a different version, and 1 for errors.

## Impact
- Affected specs: update
- Affected code: tilrd CLI, updater, integration tests and update documentation
