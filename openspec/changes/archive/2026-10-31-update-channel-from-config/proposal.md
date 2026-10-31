## Why
An implicit stable channel can replace an installed candidate unexpectedly. Persisting the channel in server.toml lets operators choose it deliberately.

## What Changes
- Read update_channel from server.toml, defaulting to stable.
- Remove the updater channel flag and reject invalid configuration before CDN access.
- Preserve signed channel validation and equality-only version comparison.

## Impact
- The update command, configuration, integration tests and release documentation.
