## Why

A new release branch inherits the old manifest from main, while the previous release commit belongs to another branch. Conventional Commit bumps can propose an existing version and repeat already shipped changelog entries.

## What Changes

- Derive the first stable version and subsequent patch versions from release/X.Y.
- Select the highest stable tag in that line, or the latest earlier stable line, as the shipped baseline. Ignore candidate tags and exclude commits already shipped, including cherry-picked fixes.
- Use the same release-please engine as action v5 through a small API runner so baseline and version overrides stay local to the run. Preserve release outputs and publishing.

## Impact

- Release workflow and deterministic tests.
- No release commit is merged or copied into main.
