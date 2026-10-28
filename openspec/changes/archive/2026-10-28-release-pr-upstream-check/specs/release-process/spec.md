## ADDED Requirements

### Requirement: Release pull requests enforce upstream first
Pull requests into `release/**` SHALL report a `release-upstream` check. Each
commit introduced relative to the target branch SHALL be reachable from main or
match a non-merge commit in main by stable Git patch identity. Only release-please
PRs SHALL be exempt. The check SHALL execute main's trusted code, treat candidate
objects only as data and fail closed on missing history.

**Verified by**: `release::release_upstream_checks_real_git_histories` (runs
`scripts/test_release_upstream.py`), `release::release_upstream_workflow_is_trusted`.

#### Scenario: Direct release fix
- **WHEN** a PR into `release/0.3` contains a patch absent from main
- **THEN** `release-upstream` fails and identifies the commit to land in main first

#### Scenario: Unsupported commits
- **WHEN** a proposed merge or empty commit is not reachable from main
- **THEN** the check fails, identifies the commit and instructs the author to land the change in main first

#### Scenario: Cherry-picked upstream fixes
- **WHEN** every proposed commit is in main or is a patch-equivalent cherry-pick from main, even with a different SHA and without a trailer
- **THEN** the check succeeds

#### Scenario: Mixed and forged commits
- **WHEN** a PR contains an upstream cherry-pick followed by a novel patch, or a novel patch with a forged cherry-pick trailer
- **THEN** the check fails for the novel patch

#### Scenario: Release automation
- **WHEN** a PR is authored by `tilr-bot[bot]` with user type `Bot`, from the same repository and branch `release-please--branches--<target release branch>`
- **THEN** the check succeeds without requiring version and changelog commits in main

#### Scenario: Spoofed release branch
- **WHEN** another author or a fork uses the release-please branch name
- **THEN** the normal upstream checks still apply

#### Scenario: Incomplete or stale snapshots
- **WHEN** Git history is missing or shallow, or fetched base and head do not match the event
- **THEN** the check fails rather than authorizing the PR
