## MODIFIED Requirements

### Requirement: release-please opens the release PR
On pushes to `release/**`, never on `main`, release-please (`simple` strategy, `release-please-config.json`, as the `tilr-bot` GitHub App) SHALL keep a release PR into the pushed branch (`target-branch`) open from the `feat` / `fix` Conventional Commits, bumping `[workspace.package] version` in `Cargo.toml`, the workspace crates' versions in `Cargo.lock`, and `CHANGELOG.md`. Versions and the changelog SHALL NOT be edited by hand. The release PR SHALL pass the same required checks as any PR.

**Verified by**: `release::release_please_bumps_the_workspace_version_and_lock` (static: `release-please-config.json`, `.github/workflows/release.yml`).

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
