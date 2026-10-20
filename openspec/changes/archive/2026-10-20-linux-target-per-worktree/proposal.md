## Why

TILR-178 records cross-worktree Linux build failures on 2026-10-20: all runs currently mount the same cargo target volume. Concurrent workers must keep their build outputs separate.

## What changes

- Give each physical checkout path its own persistent target volume, still separated by platform.
- Run the exact image produced by each invocation, avoiding a concurrently rebuilt mutable tag.
- Document volume naming and removal; exercise overlapping builds with a controlled barrier.

## Out of scope

- Isolation of two invocations in the same worktree; callers already serialize those runs.
- Changing the shared cargo registry, release layout, protocol, reload or update state.

No protocol bump or migration is needed. The old target volume becomes unused and can be removed manually. Reload, reconnect and upgrades are unaffected.
