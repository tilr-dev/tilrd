## 1. Diagnosis

- [ ] 1.1 Establish and deterministically reproduce the state-formats timeout cause.
- [ ] 1.2 Specify any resulting observable probe behavior change before implementation.

## 2. Fix and regression

- [x] 2.1 Add subprocess regressions for failed async/blocking log waits and poisoned-log cleanup; demonstrate failure before the fix.
- [x] 2.2 Release log guards before assertions and tolerate poisoned logs during cleanup.
- [ ] 2.3 Fix the established state-formats cause and prove the command-line rejection path is reached.

## 3. Review

- [ ] 3.1 Run narrow regressions, obtain a fresh-context self-review, then bin/check and bin/finish.
