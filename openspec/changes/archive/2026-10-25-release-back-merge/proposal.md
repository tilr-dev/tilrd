## Why

The owner chose GitFlow on 2026-10-25: promoting dev to main and returning release commits to dev both use merge commits. Without the return merge, the version and changelog committed on main do not reach dev automatically.

## What Changes

- Add a release workflow job that opens or reuses a main-to-dev PR as tilr-bot and attempts a merge commit after release creation.
- Leave the PR open and the release successful when the merge fails.
- Document promotion and automatic back-merge in README, and verify the workflow contract and shell behavior in release tests.

## Out of scope

- Repository rulesets and required-check policy.
- Release packaging, CDN publishing and runtime behavior.

## Compatibility

No breaking change, wire protocol change or feature flag. No runtime state is added; reloads, reconnects and updates from earlier releases are unaffected.
