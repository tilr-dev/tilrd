## MODIFIED Requirements

### Requirement: release-please opens the release PR
On pushes to `release/**`, never on `main`, release-please (`simple` strategy, `release-please-config.json`, as the `tilr-bot` GitHub App) SHALL keep a release PR into the pushed branch (`target-branch`) open from the `feat` / `fix` Conventional Commits, bumping `[workspace.package] version` in `Cargo.toml`, the workspace crates' versions in `Cargo.lock`, and `CHANGELOG.md`. Versions and the changelog SHALL NOT be edited by hand. The release PR SHALL pass the same required checks as any PR.

**Verified by**: `release::release_please_bumps_the_workspace_version_and_lock` (static: `release-please-config.json`, `.github/workflows/release.yml` and the pinned `tilr-dev/.github` release-branch workflow).

#### Scenario: A feat commit lands on main
- **WHEN** a `feat:` commit is pushed to `main` at version 0.2.1
- **THEN** no release PR is opened or updated
- **AND** once `release/0.3` is cut from that commit and pushed, the release PR into `release/0.3` proposes 0.3.0 from the branch line

#### Scenario: A new release branch follows another line
- **GIVEN** `v0.1.0` exists on `release/0.1` and its release commit is absent from `main`
- **WHEN** a new feature lands in `main` and `release/0.2` is cut from it
- **THEN** the release PR proposes 0.2.0 and its changelog contains the new feature only, excluding changes already shipped in 0.1.0

#### Scenario: A new line contains only a fix
- **GIVEN** the inherited manifest says 0.2.1
- **WHEN** only a `fix:` commit is added and `release/0.3` is cut
- **THEN** the release PR proposes 0.3.0

#### Scenario: A feature on an existing line
- **GIVEN** stable `v0.3.2` exists in the history of `release/0.3`
- **WHEN** a new `feat:` commit is pushed to that branch
- **THEN** the release PR proposes 0.3.3, even if another line has a newer stable release

#### Scenario: A fix already shipped by cherry-pick
- **GIVEN** a fix landed in `main` and its cherry-pick shipped on the previous release line
- **WHEN** the next release branch is cut from `main`
- **THEN** the changelog excludes shared history and that patch-equivalent fix

#### Scenario: Candidate tags do not select stable versions
- **GIVEN** candidate tags exist on a release line
- **WHEN** the runner selects its next version and changelog baseline
- **THEN** only stable tags are considered: the highest stable tag of the same line, otherwise the highest earlier stable version
- **AND** the first release is `X.Y.0`, and subsequent releases increment the highest stable patch

#### Scenario: A stable tag is outside the same-line history
- **GIVEN** a stable tag exists for the target release line outside the branch history
- **WHEN** the runner prepares a release
- **THEN** it fails before creating a release PR or publishing

#### Scenario: No earlier stable release exists
- **GIVEN** no earlier stable tag exists
- **WHEN** the runner prepares the first release
- **THEN** the configured bootstrap SHA bounds the changelog history

#### Scenario: Runtime overrides remain local
- **WHEN** the runner selects a version and changelog baseline
- **THEN** it copies no release commits or version files into `main`

#### Scenario: Shared orchestration
- **WHEN** a release branch is pushed
- **THEN** the caller invokes `tilr-dev/.github/.github/workflows/release-branch.yml` at a fixed commit for branch validation, release-please and candidate selection
- **AND** artifact builds and publication remain in this repository

### Requirement: Releases are cut from release branches
Nothing SHALL be released from `main`, the only integration branch: on the owner's explicit instruction a release branch `release/X.Y` is cut from a commit of `main`, and only pushes to `release/**` SHALL run release-please, release candidates and publishing. A fix for a release branch SHALL land in `main` first through a pull request and then be cherry-picked into `release/X.Y`; nothing SHALL be merged from a release branch back into `main`.

**Verified by**: `release::releases_run_only_on_release_branches` (static: `.github/workflows/release.yml` and the pinned shared workflow).

#### Scenario: A pull request is merged into main
- **WHEN** a pull request is merged into `main`
- **THEN** the release workflow does not run: no release PR, release candidate, GitHub release or CDN upload

#### Scenario: A fix for a release
- **WHEN** a fix is needed for `release/0.3`
- **THEN** it is merged into `main` through a pull request first and then cherry-picked into `release/0.3`
- **AND** no workflow merges `release/0.3` or a release back into `main`

#### Scenario: Shared orchestration
- **WHEN** a release branch is pushed
- **THEN** the caller invokes `tilr-dev/.github/.github/workflows/release-branch.yml` at a fixed commit for branch validation, release-please and candidate selection
- **AND** artifact builds and publication remain in this repository

### Requirement: A push to a release branch builds a release candidate
A push to `release/X.Y` that creates no release SHALL tag the pushed commit `vX.Y.Z-rc.N` and create a GitHub prerelease with the same archives and `.sha256` per target as a release, built and checked as a release is, the binaries naming `X.Y.Z-rc.N-<commit>`. The CDN SHALL receive the candidate archives and checksums and update only the beta pointers; latest and install.sh SHALL NOT be touched.

**Verified by**: `candidate.test.cjs` in the pinned shared workflow (the candidate script with a stub GitHub CLI), `release::a_release_candidate_is_built_like_a_release_and_publishes_to_beta` (static: `.github/workflows/release.yml` and the pinned shared workflow).

#### Scenario: The first candidate
- **WHEN** `release/0.3` is pushed and no `v0.3.*` tag exists
- **THEN** `Z` is 0 until a stable `vX.Y.*` tag exists, then one more than its highest patch; `N` is one more than the highest `vX.Y.Z-rc.*`
- **AND** the pushed commit is tagged `v0.3.0-rc.1` and a GitHub prerelease `v0.3.0-rc.1` gets `tilr-v0.3.0-rc.1-<target>.tar.gz` and its `.sha256` for every enabled target

#### Scenario: A cherry-pick after a candidate
- **WHEN** a fix is cherry-picked into `release/0.3` and `v0.3.0-rc.1` and `v0.3.0-rc.2` exist
- **THEN** the new commit is tagged `v0.3.0-rc.3`

#### Scenario: A candidate after a release
- **WHEN** `v0.3.0` is released and a fix is later cherry-picked into `release/0.3`
- **THEN** the new commit is tagged `v0.3.1-rc.1`

#### Scenario: A re-run on the same commit
- **WHEN** the workflow is re-run for a commit that already carries a candidate tag of its line, even after a later release
- **THEN** that tag is reused and no new candidate is tagged

#### Scenario: A re-run on the release commit
- **WHEN** the workflow is re-run for the commit tagged `v0.3.0` after its build or upload failed
- **THEN** no candidate is tagged and nothing is built in place of the release

#### Scenario: The push creates a release
- **WHEN** the push is the merge of the release PR and release-please creates `v0.3.0`
- **THEN** no candidate is tagged; the release is built and published to the CDN

#### Scenario: A branch not named release/X.Y
- **WHEN** a branch `release/next` is pushed
- **THEN** the release workflow fails with `release branches are named release/X.Y` before release-please runs: no release PR, release, candidate or build

#### Scenario: The CDN is left alone
- **WHEN** a release candidate is built, its prerelease has its assets and the stable CDN channel already exists
- **THEN** the stable CDN channel is left alone: the `cdn` job publishes beta and no `latest` or `install.sh` changes

#### Scenario: Shared orchestration
- **WHEN** a release branch is pushed
- **THEN** the caller invokes `tilr-dev/.github/.github/workflows/release-branch.yml` at a fixed commit for branch validation, release-please and candidate selection
- **AND** artifact builds and publication remain in this repository
