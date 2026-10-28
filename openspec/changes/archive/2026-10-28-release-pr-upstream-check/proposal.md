## Why

Pull requests into release branches currently run CI without enforcing that their
changes landed in main first. Direct fixes can therefore bypass integration and
specification authorization.

## What Changes

- Add a trusted `release-upstream` check for pull requests into `release/**`.
- Match each proposed commit against main by ancestry or stable Git patch identity.
- Exempt release-please's own release PR, identified by its bot author and source.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `release-process`: enforce upstream-first on release PRs.

## Impact

- Add one requirement to release-process and a workflow with a Git-based checker.
- Keep specification authorization on main, where upstream fixes are approved.
- The repository owner configures the required check on release branches.
