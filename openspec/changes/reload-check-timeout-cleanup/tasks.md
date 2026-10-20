## 1. Diagnosis and agreed scope

- [x] 1.1 Investigate the original state-formats timeout; record that its cause remains unproven.
- [x] 1.2 Record Dan's approval to review the cleanup fix and retain the timeout in known-flakes (2026-10-20, TILR-170 #3750). No product behavior change is proposed.

## 2. Fix and regression

- [x] 2.1 Add subprocess regressions for failed async/blocking log waits and poisoned-log cleanup; demonstrate failure before the fix.
- [x] 2.2 Release log guards before assertions and tolerate poisoned logs during cleanup.
- [x] 2.3 Verify the existing command-line rejection test reaches its assertion without changing the probe.
