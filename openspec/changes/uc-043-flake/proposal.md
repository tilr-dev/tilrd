# Proposal: Fix the itests uc_043 flake

## Why

Known flake (Basic Memory `main/projects/tilr/tilrd-tests-integratsionnye-testy-tilrd-na-dagger-use-cases-resheniia`): itests `uc_043` p90 echo over 5 s in 2 of about 7 runs. A flake is closed only with its cause established.

## What Changes

- Find the cause (load from repeats, the proxy or the server), reproduce it deterministically on a stand that sets the load or delay itself (TILR-160), not with `--repeat`; fix it, or quarantine with the cause recorded in itests/USE_CASES.md.

## Capabilities

None: tests only, no requirement changes.

## Impact

`itests/scenarios/uc_043.py` and the itests stand. No behavior, protocol, reload or update impact. Tracker: TILR-152 (epic TILR-76). Split from the former shared change `test-coverage-gaps` (2026-10-20: one task, one change).
