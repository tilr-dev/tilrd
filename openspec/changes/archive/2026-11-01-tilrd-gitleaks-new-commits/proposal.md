## Why
Scanning already protected base history on every pull request and merge group duplicates the nightly scan. New commits need a required secrets gate, while periodic scans retain coverage of existing history.

## What Changes
- Scan base..HEAD for pull requests and merge queue groups.
- Keep full checked-commit history for release pushes, nightly runs and manual runs.
- Update the release-process requirement and static CI regression coverage.

## Impact
- Affected spec: release-process.
- Affected code: CI workflow and release workflow tests.
