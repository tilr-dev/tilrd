## Why

The existing workflow selects stable releases through commit-derived release pull requests. The approved release model selects stable versions explicitly with tags, while branch pushes create candidates.

## What Changes

- Call the pinned shared tag/candidate workflow and retain signing, notarization, CDN and desktop pin publication.
- Remove release automation configuration and its upstream-check exemption.
- Stamp builds with the selected tag and validate that the numeric tag version is at least the Cargo version.
- Update workflow tests and release documentation.

## Capabilities

### Modified Capabilities
- `release-process`: tag-triggered stable releases and branch-triggered candidates.
- `itest-port-stand`: release fixtures without obsolete configuration.

## Impact

No wire protocol, runtime state, reload or reconnect change. Updates keep the existing archive/CDN contract.

## Non-goals

- Changing signing or CDN formats.
- Changing repository deployment policies or tag rulesets.
