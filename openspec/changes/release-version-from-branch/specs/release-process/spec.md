## MODIFIED Requirements

### Requirement: release-please opens the release PR
On pushes to `release/**`, never on `main`, release-please (`simple` strategy, `release-please-config.json`, as the `tilr-bot` GitHub App) SHALL keep a release PR into the pushed branch (`target-branch`) open from the `feat` / `fix` Conventional Commits, bumping `[workspace.package] version` in `Cargo.toml`, the workspace crates' versions in `Cargo.lock`, and `CHANGELOG.md`. Versions and the changelog SHALL NOT be edited by hand. The release PR SHALL pass the same required checks as any PR.

**Verified by**: `release::release_please_bumps_the_workspace_version_and_lock` (static: `release-please-config.json`, `.github/workflows/release.yml`).

#### Scenario: A feat commit lands on main
- **WHEN** a `feat:` commit is pushed to `main` at version 0.2.1
- **THEN** no release PR is opened or updated
- **AND** once `release/0.3` is cut from that commit and pushed, the release PR into `release/0.3` proposes 0.3.0 from the branch line

The release version SHALL be `X.Y.0` for the first stable release of `release/X.Y`, then the highest stable `vX.Y.Z` plus one patch. Candidate tags SHALL NOT select the version or baseline. The changelog SHALL include only changes not shipped in the highest stable tag of that line, or the highest earlier stable version when this line has none. The runner SHALL exclude shared history and patch-equivalent cherry-picked changes. An existing same-line stable tag outside the branch history SHALL fail safely. With no earlier stable tag, the configured bootstrap SHA SHALL bound the history. The runtime overrides SHALL NOT copy release commits or version files into `main`.

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
- **THEN** the changelog excludes that patch-equivalent fix
