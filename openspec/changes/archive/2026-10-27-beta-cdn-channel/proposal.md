## Why
Release candidates already have GitHub prereleases, but test machines need the normal updater to install them from a separate CDN channel before promotion to stable.

## What Changes
- Publish candidate archives and checksums before the platform beta pointer, without changing latest or install.sh.
- Add tilrd update --channel stable|beta, defaulting to stable; no environment variable.
- Keep stable publication unchanged, including leaving beta alone.

## Capabilities
### New Capabilities
None.
### Modified Capabilities
- release-process: publish release candidates to the beta CDN channel.
- update: select the stable or beta pointer.

## Impact
Release workflow, release packaging script, daemon updater, integration tests and release documentation.
