## Why
The README at commit 9e8e4b22 explains selecting beta and stable but does not explicitly describe returning to an older stable release after testing an RC.

## What Changes
- Document that switching back to stable installs its current release even when older than the installed RC, including app-triggered updates, in-place reload and the check command.
- Add the corresponding scenario to the release-channel selection requirement.

## Capabilities
### New Capabilities
None.
### Modified Capabilities
- update: explicitly describe rollback after switching from beta to stable.

## Impact
Documentation only. Existing update and reload behavior remains unchanged, including shell preservation across reload and updates from the previous release. No protocol version change or feature flag is needed.

## Non-goals
- Change updater code or introduce downgrade protection or a force flag.
