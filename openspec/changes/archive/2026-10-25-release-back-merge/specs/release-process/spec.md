## ADDED Requirements

### Requirement: A created release automatically back-merges main into dev
The release workflow SHALL back-merge `main` into `dev` only when release-please reports `release_created == 'true'`, using a repository-scoped tilr-bot GitHub App token. It SHALL open a PR titled `chore: back-merge vX.Y.Z` using the release tag, or reuse an existing open main-to-dev PR, and attempt `gh pr merge --merge`. A failed merge SHALL leave the PR open without failing the release.

**Verified by**: `release::a_created_release_back_merges_main_into_dev` (static workflow contract), `release::back_merge_creates_or_reuses_a_pr_and_keeps_conflicts_open` (workflow shell with a stub GitHub CLI).

#### Scenario: A release is created
- **WHEN** release-please creates release `v1.2.3` and no main-to-dev PR is open
- **THEN** tilr-bot opens `main` to `dev` titled `chore: back-merge v1.2.3` and merges it with a merge commit

#### Scenario: A back-merge PR is already open
- **WHEN** a release is created and a main-to-dev PR is already open
- **THEN** the workflow attempts to merge that PR without opening another PR

#### Scenario: A merge conflicts
- **WHEN** the back-merge PR cannot be merged because of a conflict
- **THEN** the PR remains open for manual resolution and the release remains successful

#### Scenario: No release is created
- **WHEN** release-please does not report a created release
- **THEN** the workflow skips the back-merge job
