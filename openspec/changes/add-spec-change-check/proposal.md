## Why

Existing specification requirements need explicit authorization before a pull request can change them. A required check on dev enforces the same rule for automated and human changes.

## What Changes

- Add a required spec-change check for pull requests into dev.
- Share one checker across repositories, also usable by local merge automation.
- Read task comments with the dedicated read-only CI credential.
- Keep other tests local for dev; preserve main checks.

## Capabilities

### Modified Capabilities
- release-process: add specification authorization checks.

## Impact

GitHub Actions, the dev ruleset, task comment reads, and local verification tooling.
