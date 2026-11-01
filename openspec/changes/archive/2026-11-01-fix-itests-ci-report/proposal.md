# Reliable integration test reports

## Why
The CI integration suite can finish running cases and then fail while exporting its
report, leaving neither a summary nor the Dagger error in the job log.

## What Changes
- Export complete reports with large aggregate test logs.
- Preserve Dagger diagnostics when the report export fails.
- Cover report export and wrapper failure handling with deterministic regressions.

## Impact
- Affected spec: itest-port-stand (ADDED requirements).
- Affected code: integration report construction and the dt wrapper.
