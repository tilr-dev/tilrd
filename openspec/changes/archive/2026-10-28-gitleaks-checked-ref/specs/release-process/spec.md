## ADDED Requirements

### Requirement: CI scans secrets in the checked commit's history only
The CI gitleaks gate SHALL scan the whole history of the commit under check (the pushed commit, the pull request's test merge or the merge queue group's commit) and no other branch fetched with it, so that a finding on another unmerged branch does not fail the check.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A leak on another pull request branch
- **WHEN** a commit with a gitleaks finding exists only on an unmerged pull request branch and `main` is pushed
- **THEN** the gitleaks job on `main` scans only `main`'s history and passes

#### Scenario: A leak in the checked history
- **WHEN** the pull request under check carries a commit with a gitleaks finding anywhere in its history
- **THEN** the gitleaks job fails
