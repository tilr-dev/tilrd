## Why

The desktop application no longer pins a tilrd release: it installs its bundled tilrd and tilr only when none is installed and then keeps the installed pair on the latest release through `tilrd update`. A bump pull request could only reach users with the next application release, and it could not merge without a spec change, so a desktop release shipped an older server than the latest one.

## What Changes

- Stop opening the desktop server pin pull request after a stable release.
- Remove the bump script, its tests and the documentation of the bot's desktop-mac access.

## Capabilities

### Modified Capabilities
- `release-process`

## Impact

The tilrd release workflow, its tests and release documentation. The bot no longer needs write access to desktop-mac for releases.
