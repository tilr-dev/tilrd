## Why

Desktop releases currently need a manual upstream server pin update. A stable server release should propose its signed artifacts for the next desktop release through the normal review and CI process.

## What Changes

- After stable GitHub release assets are uploaded, open or update one bot pull request into desktop-mac main.
- Read the protocol from the released source metadata, preserve the pin format and skip release candidates.
- Document the bot installation permissions and the desktop release flow.

## Capabilities

### Modified Capabilities
- `release-process`

## Impact

The tilrd release workflow and its integration tests; desktop-mac production documentation. Bot installation access to desktop-mac is required.
