## MODIFIED Requirements

### Requirement: CI scans secrets in the checked commit's history only
The CI gitleaks gate SHALL scan `pull_request.base.sha..HEAD` for pull requests into `main` or `release/**` (the test merge), and `merge_group.base_sha..HEAD` for merge queue groups: only commits reachable from HEAD but not the event's base. For pushes to `release/**`, nightly runs on `main` and manual runs, it SHALL scan the whole history of `HEAD`. No scan SHALL include another branch merely because checkout fetched it.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: Pull request scans new commits
- **WHEN** a pull request into `main` or `release/**` runs CI
- **THEN** gitleaks scans `pull_request.base.sha..HEAD`, including the test merge and all commits introduced by the pull request
- **AND** findings reachable only from the base do not fail this gate

#### Scenario: Merge queue scans the group's new commits
- **WHEN** a merge queue group runs CI
- **THEN** gitleaks scans `merge_group.base_sha..HEAD`, including all commits introduced by the group
- **AND** findings reachable only from the group's base do not fail this gate

#### Scenario: A leak on another pull request branch
- **WHEN** a commit with a gitleaks finding exists only on an unmerged pull request branch and the merge queue checks a group for `main`
- **THEN** the gitleaks job excludes that branch and passes

#### Scenario: A leak in a new commit
- **WHEN** a pull request or merge queue group introduces a commit with a gitleaks finding, even if a later commit removes the secret
- **THEN** the gitleaks job fails

#### Scenario: Periodic and manual full-history scans
- **WHEN** the nightly or manual CI run checks `main`
- **THEN** gitleaks scans the whole history reachable from `HEAD` and fails on a finding anywhere in that history

#### Scenario: Release push preserves full-history coverage
- **WHEN** a commit is pushed to `release/**`
- **THEN** gitleaks scans the whole history reachable from that commit and fails on a finding anywhere in that history
