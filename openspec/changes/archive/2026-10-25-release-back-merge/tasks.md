## Implementation

- [x] Add an automatic back-merge requirement with creation, reuse, conflict and no-release scenarios; verify with OpenSpec validation.
- [x] Add the bot-authenticated release job and README promotion instructions; verify with `release::a_created_release_back_merges_main_into_dev` and `release::back_merge_creates_or_reuses_a_pr_and_keeps_conflicts_open`.
- [x] Record test coverage in USE_CASES.md and regenerate local coverage; verify with `./itests/dt coverage`.
