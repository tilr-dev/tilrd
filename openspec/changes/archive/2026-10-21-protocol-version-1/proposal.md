# Protocol version 1 until the first public release

## Why

The owner confirmed on 2026-10-21 that there has been no public release and that ongoing development must use protocol version 1.

## What Changes

- **BREAKING**: reset the mandatory wire contract from version 3 to version 1. Migration: rebuild clients against the shared version 1 schema; other versions are refused.
- Keep all mandatory capabilities in one contract, with no new feature flag.
- Before the first public release, all protocol changes stay on version 1 without a bump. Afterward, bump only for incompatibility with a client or server of the previous public release, once per release as a separate step in its release milestone.
- Align the schema header, README, project guidance and protocol scenarios.

## Capabilities

### Modified Capabilities
- `protocol`: reset the version check and rename the mandatory contract.

## Non-goals

- Updating the external desktop app or server plugin.
- Implementing a release automation mechanism or changing wire messages.

## Reload and Update

No reload state format changes. A reconnect uses version 1; clients built for another version must be rebuilt. There is no previous public release to migrate from.
