# Bind specification authorization to task metadata

## Why
The checker returns success when intake scope is absent, before validating task age or status. It also collects authorization blocks without checking the task lifecycle. Selecting another task number in a branch can therefore reuse a legacy exception or stale grants.

## What changes
- Require trusted task metadata alongside complete comments.
- Limit legacy success to tasks created before the original rule landed in dev, with trusted intake ready and neither Done nor Canceled.
- Count grants only in Review, Supervise, or Merge; rerunning after a column transition evaluates the current state.
- Apply the same shared-checker contract in all component specifications.

## Out of scope
- Deployment of the trusted checker and pinned workflow digests in main.
- Changes to review or merge bot intake rules, runtime protocols, reload, or update behavior.
