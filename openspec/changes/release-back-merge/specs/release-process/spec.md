## ADDED Requirements

### Requirement: A created release automatically back-merges main into dev
After release-please reports `release_created == 'true'`, the release workflow SHALL use a repository-scoped tilr-bot GitHub App token to open a PR from `main` to `dev` titled `chore: back-merge vX.Y.Z`, using the created release's tag. It SHALL reuse an existing open PR from `main` to `dev` instead of creating a duplicate, and attempt to merge the PR with `gh pr merge --merge`. If merging fails, including because of a conflict, it SHALL leave the PR open without failing the release. When no release is created, it SHALL skip the back-merge job.

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
