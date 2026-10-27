# Report specification authorization for merge groups

## Why
The merge queue waits for every required check on its temporary merge commit. Specification authorization currently reports only on pull requests, so the queue cannot proceed.

## What Changes
- Trigger the existing spec-change job on merge_group checks_requested.
- Report queue success without repeating the authorization already required for admission.
- Keep pull request authorization and its trusted checker unchanged.

## Impact
- Specification change authorization on main.
- The specification authorization workflow and its event routing tests.
