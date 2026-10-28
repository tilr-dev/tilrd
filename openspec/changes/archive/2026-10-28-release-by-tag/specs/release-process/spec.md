## MODIFIED Requirements

### Requirement: CI gates every change
Every pull request into `main` or `release/**`, every merge queue group and every push to `main` or `release/**` SHALL pass `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo nextest run --workspace --features tilrd/failpoints --locked` on Linux and macOS 15; `cargo check` with failpoints at the MSRV 1.99; `cargo deny check`; `gitleaks git` over the whole history.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

#### Scenario: A release branch fix is checked
- **WHEN** a fix pull request into `release/0.3` is opened or updated
- **THEN** the same checks run on it as on a pull request into `main`

#### Scenario: Build at the minimum supported Rust version
- **WHEN** CI runs the MSRV job
- **THEN** it installs Rust 1.99 and runs `cargo check --workspace --all-targets --features tilrd/failpoints --locked`

### Requirement: The CDN gets the release last
After GitHub assets are attached, the `cdn` job SHALL verify each built platform carries `<tag version>-<commit>` and sign manifests and upload its completed `dist/cdn` layout with `scripts/release.sh --upload-only --channel <channel> <platforms>`, for both stable tag releases and candidates. The channel SHALL be stable for a release and beta for a candidate. Both macOS platforms SHALL always be included; Linux arm64 SHALL remain optional.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** the `macos-aarch64`, `macos-x86_64` and `linux-x86_64` archives, checksums and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

#### Scenario: macOS publication
- **WHEN** both macOS builds complete and GitHub assets are attached
- **THEN** the CDN job uploads each macOS archive and checksum before that platform's selected channel pointer (latest for stable, beta for a candidate), without rebuilding or re-signing

#### Scenario: CDN credentials
- **WHEN** the CDN job publishes a release
- **THEN** it uses environment `cdn`, restricted to `release/*` branches and `v*` tags, with bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets and the `CLOUDFLARE_ACCOUNT_ID` variable. The signing step SHALL use the `TILR_UPDATE_SIGNING_KEY` secret (Ed25519 PEM); build jobs SHALL embed the public `TILR_UPDATE_PUBLIC_KEY` repository variable and SHALL NOT receive the private key

#### Scenario: Candidate publication
- **WHEN** candidate GitHub assets have been attached
- **THEN** the CDN job uploads completed archives and checksums before signed beta manifests and beta for enabled platforms, without rebuilding or modifying latest or install.sh

### Requirement: Releases are cut from release branches
Nothing SHALL be released from `main`. Branch pushes to `release/X.Y` SHALL create candidates; owner-created stable `vX.Y.Z` tag pushes SHALL release only commits in the matching branch history. Fixes SHALL land in main first and then be cherry-picked; release branches SHALL NOT be merged back into main.

**Verified by**: `release::releases_run_only_on_release_branches` (static: `.github/workflows/release.yml` and the pinned shared workflow).

#### Scenario: Owner selects the release
- **WHEN** the owner prepares a release line
- **THEN** `release/X.Y` is cut from a commit of main on the owner's explicit instruction
- **AND** the owner creates the stable tag through `bin/release`, without requiring conventional commits, a version commit or a generated changelog

#### Scenario: A pull request is merged into main
- **WHEN** a pull request is merged into `main`
- **THEN** the release workflow does not run: no release candidate, GitHub release or CDN upload

#### Scenario: A fix for a release
- **WHEN** a fix is needed for `release/0.3`
- **THEN** it is merged into `main` through a pull request first and then cherry-picked into `release/0.3`
- **AND** no workflow merges `release/0.3` or a release back into `main`

### Requirement: A push to a release branch builds a release candidate
A push to `release/X.Y` whose commit does not already carry a stable tag on that line SHALL tag the pushed commit `vX.Y.Z-rc.N` and create a GitHub prerelease with the same archives and `.sha256` per target as a release, built and checked as a release is, the binaries naming `X.Y.Z-rc.N-<commit>`. The CDN SHALL receive the candidate archives and checksums and update only the beta pointers; latest and install.sh SHALL NOT be touched.

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

#### Scenario: Candidate tags do not retrigger stable builds
- **WHEN** automation creates `v0.3.0-rc.1`
- **THEN** the caller excludes that tag push and does not invoke stable validation

#### Scenario: A branch not named release/X.Y
- **WHEN** a branch `release/next` is pushed
- **THEN** the release workflow fails with `release branches are named release/X.Y` before any tag or release mutation: no release, candidate or build

#### Scenario: The CDN is left alone
- **WHEN** a release candidate is built, its prerelease has its assets and the stable CDN channel already exists
- **THEN** the stable CDN channel is left alone: the `cdn` job publishes beta and no `latest` or `install.sh` changes

#### Scenario: Shared orchestration
- **WHEN** a release branch is pushed
- **THEN** the caller invokes `tilr-dev/.github/.github/workflows/release-branch.yml` at a fixed full commit with the same implementation pin for validation and candidate selection
- **AND** artifact builds and publication remain in this repository


### Requirement: Release pull requests enforce upstream first
Pull requests into `release/**` SHALL report a `release-upstream` check. Each
commit introduced relative to the target branch SHALL be reachable from main or
match a non-merge commit in main by stable Git patch identity. No author or source branch SHALL be exempt. The check SHALL execute main's trusted code, treat candidate
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

#### Scenario: Automation follows upstream first
- **WHEN** an automation-authored pull request proposes a patch absent from main
- **THEN** the normal upstream checks fail for that patch

#### Scenario: Incomplete or stale snapshots
- **WHEN** Git history is missing or shallow, or fetched base and head do not match the event
- **THEN** the check fails rather than authorizing the PR

### Requirement: Stable releases propose the desktop server pin
After stable GitHub assets are uploaded, the release workflow SHALL use tilr-bot to propose the exact version and released protocol in `tilr-dev/desktop-mac` main through one pin pull request. It SHALL preserve the pin's positive artifact revision, run ordinary desktop CI and require ordinary review and merge. Candidates SHALL NOT propose updates.

#### Scenario: Stable release
- **WHEN** GitHub assets for stable `vX.Y.Z` have been uploaded on a stable tag push with a nonempty tag and `is_rc=false`
- **THEN** tilr-bot opens the pin update pull request into desktop-mac main
- **AND** its title is `chore: bump tilrd to X.Y.Z`
- **AND** `server-release.json` selects `X.Y.Z` and `PROTOCOL_VERSION` from the released source, preserving `build` from main
- **AND** ordinary desktop CI acquires that exact release through `fetch_server.py`

#### Scenario: Candidate or no release
- **WHEN** a release candidate is built or the shared workflow selects no tag
- **THEN** no desktop pin pull request or branch is created or updated

#### Scenario: An update pull request is already open
- **WHEN** another stable release is published newer than both main and the proposed pin while the bot pin pull request is open
- **THEN** the same pull request is updated from current main with the new pin, title and description
- **AND** no second pull request is opened

#### Scenario: Missing or invalid release metadata
- **WHEN** the released protocol metadata is unavailable or invalid
- **THEN** the update fails without publishing a guessed pin

#### Scenario: Main already pins the release
- **WHEN** main already has the same version and protocol
- **THEN** no formatting-only change or new pull request is published

#### Scenario: Older or equal stable release after a newer pin
- **WHEN** a stable release is not newer by numeric major, minor and patch than the version pinned on desktop main or the version proposed by an open bot pin pull request
- **THEN** the step logs the skipped release and exits successfully
- **AND** the remote bot branch and pull request remain unchanged
- **AND** a leftover bot branch without an open pull request does not prevent a newer-than-main release from opening a new pull request

## REMOVED Requirements

### Requirement: release-please opens the release PR
**Reason**: Stable releases are explicitly selected by tags.
**Migration**: Use `bin/release` to push the stable tag.

### Requirement: Merging the release PR builds and attaches the binaries
**Reason**: The stable build is triggered by a tag.
**Migration**: See A release tag builds and attaches the binaries.

## ADDED Requirements

### Requirement: A release tag builds and attaches the binaries
A push of the owner-created stable tag `vX.Y.Z` SHALL create a GitHub release with `tilrd` and `tilr` archives and checksums for every enabled target. Native builds SHALL have their version checked against the tag and commit. macOS builds SHALL be Developer ID signed and accepted by Apple notarization before packaging.

**Verified by**: `release::a_release_builds_and_packages_every_target` (static: `.github/workflows/release.yml`, `scripts/package.sh`).

#### Scenario: Release v0.2.0
- **WHEN** stable tag `v0.2.0` is pushed from the history of `release/0.2`
- **THEN** tag `v0.2.0` has a GitHub release with one `tilr-v0.2.0-<target>.tar.gz` and checksum per target, each binary saying `0.2.0-<commit>`

#### Scenario: Apple rejects a build
- **WHEN** signing, signature verification or notarization fails for either macOS target
- **THEN** the release workflow fails and neither GitHub assets nor CDN latest pointers are published by the release workflow

#### Scenario: Release target selection and packaging
- **WHEN** the release build matrix runs
- **THEN** it builds `aarch64-apple-darwin` and cross-compiles `x86_64-apple-darwin` on macOS, and builds `x86_64-unknown-linux-gnu` plus `aarch64-unknown-linux-gnu` only with `RELEASE_LINUX_ARM64=true`
- **AND** Linux platforms are built through `scripts/release.sh`, while macOS compilation, signing and packaging run in separate jobs
- **AND** `scripts/package.sh` attaches `tilr-<tag>-<target>.tar.gz` containing `tilrd`, `tilr`, licenses and README with its `.sha256`

#### Scenario: Stable tag outside the release line
- **WHEN** `v0.3.0` points outside the history of `release/0.3`, or that branch is missing
- **THEN** the shared workflow refuses before creating a release or building and publishing assets

#### Scenario: Version source
- **WHEN** a stable or candidate tag is built
- **THEN** checkout selects the tag and `TILR_BUILD_VERSION` is its version without `v`
- **AND** CI refuses a numeric tag version lower than the workspace Cargo version before compilation; Cargo files remain unchanged

#### Scenario: Stable notes
- **WHEN** a stable tag release is created
- **THEN** GitHub generates release notes from the highest previous stable tag on the line, or the highest previous stable tag overall when the line has none
- **AND** the first release omits the baseline; no tracked changelog is generated
