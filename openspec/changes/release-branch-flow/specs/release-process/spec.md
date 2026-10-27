## ADDED Requirements

### Requirement: Releases are cut from release branches
Nothing SHALL be released from `main`, the only integration branch: on the owner's explicit instruction a release branch `release/X.Y` is cut from a commit of `main`, and only pushes to `release/**` SHALL run release-please, release candidates and publishing. A fix for a release branch SHALL land in `main` first through a pull request and then be cherry-picked into `release/X.Y`; nothing SHALL be merged from a release branch back into `main`.

**Verified by**: `release::releases_run_only_on_release_branches` (static: `.github/workflows/release.yml`).

#### Scenario: A pull request is merged into main
- **WHEN** a pull request is merged into `main`
- **THEN** the release workflow does not run: no release PR, release candidate, GitHub release or CDN upload

#### Scenario: A fix for a release
- **WHEN** a fix is needed for `release/0.3`
- **THEN** it is merged into `main` through a pull request first and then cherry-picked into `release/0.3`
- **AND** no workflow merges `release/0.3` or a release back into `main`

### Requirement: A push to a release branch builds a release candidate
A push to `release/X.Y` that creates no release SHALL tag the pushed commit `vX.Y.Z-rc.N` and create a GitHub prerelease with the same archives and `.sha256` per target as a release, built and checked as a release is, the binaries naming `X.Y.Z-rc.N-<commit>`. `Z` is 0 until a stable `vX.Y.*` tag exists, then one more than its highest patch; `N` is one more than the highest `vX.Y.Z-rc.*`. The CDN SHALL NOT be touched.

**Verified by**: `release::a_push_to_a_release_branch_tags_the_next_release_candidate` (the workflow step's shell with a stub GitHub CLI), `release::a_release_candidate_is_built_like_a_release_and_skips_the_cdn` (static: `.github/workflows/release.yml`).

#### Scenario: The first candidate
- **WHEN** `release/0.3` is pushed and no `v0.3.*` tag exists
- **THEN** the pushed commit is tagged `v0.3.0-rc.1` and a GitHub prerelease `v0.3.0-rc.1` gets `tilr-v0.3.0-rc.1-<target>.tar.gz` and its `.sha256` for every enabled target

#### Scenario: A cherry-pick after a candidate
- **WHEN** a fix is cherry-picked into `release/0.3` and `v0.3.0-rc.1` and `v0.3.0-rc.2` exist
- **THEN** the new commit is tagged `v0.3.0-rc.3`

#### Scenario: A candidate after a release
- **WHEN** `v0.3.0` is released and a fix is later cherry-picked into `release/0.3`
- **THEN** the new commit is tagged `v0.3.1-rc.1`

#### Scenario: A re-run on the same commit
- **WHEN** the workflow is re-run for a commit that already carries a candidate tag of its version
- **THEN** that tag is reused and no new candidate is tagged

#### Scenario: The push creates a release
- **WHEN** the push is the merge of the release PR and release-please creates `v0.3.0`
- **THEN** no candidate is tagged; the release is built and published to the CDN

#### Scenario: A branch not named release/X.Y
- **WHEN** a branch `release/next` is pushed
- **THEN** the step fails with `release branches are named release/X.Y` and nothing is tagged or built

#### Scenario: The CDN is left alone
- **WHEN** a release candidate is built and its prerelease has its assets
- **THEN** the `cdn` job does not run and no `latest` changes

### Requirement: Specification merge readiness diagnostics
Before fetching a pull request's test merge for specification authorization, the trusted check SHALL read the pull request through the GitHub API with pull-requests read permission, with the shared helper `spec_merge.py` downloaded from a fixed gitpm commit and checked against its pinned digest. It SHALL tell a conflict with `main` from a pending or stale merge result, and SHALL inspect candidate Git objects without executing candidate code.

**Verified by**: `release::the_spec_change_check_compares_main_with_the_test_merge` (static: `.github/workflows/spec-change.yml`).

#### Scenario: Pull request conflicts with main
- **WHEN** the API reports mergeable false for a head SHA matching the requested candidate SHA
- **THEN** the check fails immediately with a conflict diagnostic naming the task branch
- **AND** it does not advise re-running the job

#### Scenario: GitHub is still computing the merge
- **WHEN** mergeable is null or the fetched merge commit does not have the requested candidate SHA as its second parent
- **THEN** the check waits and reads the API again for up to approximately 60 seconds
- **AND** it proceeds once the API head SHA matches the candidate SHA, mergeable is true and the merge commit has the candidate SHA as its second parent
- **AND** it uses the first parent of that verified merge as the comparison baseline

#### Scenario: API response describes a stale head
- **WHEN** the API head SHA differs from the requested candidate SHA
- **THEN** the check treats that response as stale and reads the API again within the existing deadline

#### Scenario: Command fails
- **WHEN** the API or Git command fails or times out
- **THEN** the check fails with a diagnostic identifying the command and preserving its error output

#### Scenario: Merge result remains stale
- **WHEN** the merge result remains pending or stale until the wait deadline
- **THEN** the check fails with the diagnostic that GitHub has not updated the merge result and advises re-running the job

## MODIFIED Requirements

### Requirement: Linux binaries are built in a Debian 12 container
`scripts/linux-build.sh` SHALL run `cargo build --release -p tilrd -p tilr` (no `failpoints`) through `scripts/linux.sh` in the image of `scripts/linux.Dockerfile` (`rust:1.99-bookworm` pinned by digest: glibc 2.36, `protobuf-compiler`, Zig 0.15.2 from `scripts/install-tools.sh`) for the architecture `TILR_LINUX_PLATFORM` names (default the host's), passing `TILR_BUILD_VERSION` and `TILR_BUILD_COMMIT` in, and copy the results to `dist/linux-<arch>/tilrd` and `dist/linux-<arch>/tilr`.

**Verified by**: `release::the_linux_build_runs_in_the_pinned_debian_12_image` (static: `scripts/linux.Dockerfile` and `scripts/linux-build.sh`).

#### Scenario: amd64 from another machine
- **WHEN** `TILR_LINUX_PLATFORM=linux/amd64 scripts/linux-build.sh` runs
- **THEN** `dist/linux-x86_64/tilrd` and `dist/linux-x86_64/tilr` are written

### Requirement: A release is built from a clean commit
`scripts/release.sh` SHALL refuse to build when `git status --porcelain` is not empty (`uncommitted changes: a release is built from a commit`, exit 1). The version SHALL be the package version, `-`, and the short commit: the package version is `TILR_BUILD_VERSION` if set and non-empty, else the root `Cargo.toml`'s first `version = "..."`. Both SHALL be passed to the build, as `TILR_BUILD_VERSION` and `TILR_BUILD_COMMIT`, so the binaries name the version.

**Verified by**: `release::a_dirty_tree_builds_no_release`, `release::a_release_is_named_after_the_package_version_and_the_commit` and `release::a_release_is_named_after_the_build_version_when_given`.

#### Scenario: Dirty tree
- **WHEN** `scripts/release.sh` runs with an uncommitted change
- **THEN** it prints `uncommitted changes: a release is built from a commit` and exits 1 without building

#### Scenario: A release candidate's version
- **WHEN** `TILR_BUILD_VERSION=0.3.0-rc.1 scripts/release.sh linux-x86_64` runs at commit `26449dd`
- **THEN** the binaries say `0.3.0-rc.1-26449dd` and `releases/linux-x86_64/latest` in `dist/cdn` is `0.3.0-rc.1-26449dd`

### Requirement: CI gates every change
Every pull request into `main` or `release/**`, every merge queue group and every push to `main` or `release/**` SHALL pass `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo nextest run --workspace --features tilrd/failpoints --locked` on Linux and macOS 15; `cargo check` with failpoints at the MSRV 1.90; `cargo deny check`; `gitleaks git` over the whole history.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

#### Scenario: The release PR is checked
- **WHEN** the release PR into `release/0.3` is opened or updated, or a fix is cherry-picked into `release/0.3`
- **THEN** the same checks run on it as on a pull request into `main`

### Requirement: release-please opens the release PR
On pushes to a release branch `release/**`, never on `main`, release-please (`simple` strategy, `release-please-config.json`, acting as the `tilr-bot` GitHub App) SHALL keep a release PR into the pushed branch (`target-branch`) open from the `feat` / `fix` Conventional Commits, bumping `[workspace.package] version` in `Cargo.toml`, the workspace crates' versions in `Cargo.lock`, and `CHANGELOG.md`. Versions and the changelog SHALL NOT be edited by hand. The release PR SHALL pass the same required checks as any PR.

**Verified by**: `release::release_please_bumps_the_workspace_version_and_lock` (static: `release-please-config.json`, `.github/workflows/release.yml`).

#### Scenario: A feat commit lands on main
- **WHEN** a `feat:` commit is pushed to `main` at version 0.2.1
- **THEN** no release PR is opened or updated
- **AND** once `release/0.3` is cut from that commit and pushed, the release PR into `release/0.3` proposes the next minor version 0.3.0 (`bump-patch-for-minor-pre-major` is false)

### Requirement: The CDN gets the release last
After GitHub assets are attached, the `cdn` job SHALL verify each built platform carries `<tag version>-<commit>` and upload its completed `dist/cdn` layout with `scripts/release.sh --upload-only <platforms>`, for a created release only, never for a release candidate. Both macOS platforms SHALL always be included; Linux arm64 SHALL remain optional.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** the `macos-aarch64`, `macos-x86_64` and `linux-x86_64` archives, checksums and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

#### Scenario: macOS publication
- **WHEN** both macOS builds complete and GitHub assets are attached
- **THEN** the CDN job uploads each macOS archive and checksum before that platform's latest, without rebuilding or re-signing

#### Scenario: CDN credentials
- **WHEN** the CDN job publishes a release
- **THEN** it uses environment `cdn`, restricted to `release/*` branches, with bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets and the `CLOUDFLARE_ACCOUNT_ID` variable

### Requirement: Specification change authorization on main
Pull requests into main SHALL pass the required spec-change check. One shared checker SHALL enforce intake Spec scope and accept authorization only from Dan's comments, using the dedicated read-only task credential. The check SHALL run only from the trusted pull_request_target workflow in main and compare main with GitHub's test merge of the pull request; a copy of that workflow in the pull request is never run. The other checks run in CI.

#### Scenario: Existing specification is changed without authorization
- **WHEN** an intake-ready comment declares Spec scope and a pull request removes or changes existing specification lines without a matching spec-change entry from Dan
- **THEN** the check fails and lists the uncovered requirements

#### Scenario: Additions and the task's own change
- **WHEN** a pull request only adds requirements or edits its own change declared by intake
- **THEN** the check passes without an authorization block
- **AND** deltas that modify existing requirements still require authorization

#### Scenario: Authorized changes
- **WHEN** every affected existing requirement and foreign change has a matching operation in Dan's spec-change block for this component
- **THEN** the check passes

#### Scenario: Bot impersonates authorization
- **WHEN** an authorization block is posted by a bot instead of Dan
- **THEN** the block does not authorize any changes

#### Scenario: Task predates scope declarations
- **WHEN** the task's intake-ready comments contain no Spec scope line
- **THEN** the check passes as a legacy task

#### Scenario: Foreign change edited or archived
- **WHEN** a pull request edits or archives another task's change
- **THEN** the matching edit or archive entry from Dan is required

#### Scenario: Task data is unavailable
- **WHEN** the task cannot be identified or its comments cannot be read completely
- **THEN** the check fails with an actionable error

#### Scenario: Local merge automation
- **WHEN** local merge automation invokes the checker with a repository, base, head, component, and task branch
- **THEN** the same authorization rules and exit status apply as in CI

#### Scenario: Candidate code cannot replace the checker
- **WHEN** a pull request changes checker scripts or workflow definitions
- **THEN** the check still runs the workflow from main, not a workflow file from the pull request
- **AND** the trusted workflow reads candidate Git objects without checking out or executing candidate files
- **AND** the shared checker content is downloaded from a fixed gitpm commit and verified against a digest pinned in main's workflow before execution, and a new commit or digest takes effect only through a pull request into main
- **AND** only the checker process receives the read-only task credential

#### Scenario: A branch behind main
- **WHEN** a task branch forked before `main` gained a requirement and the pull request leaves that requirement alone
- **THEN** the check compares `main` with GitHub's test merge of the pull request, so that requirement does not count as removed

#### Scenario: Malformed scope declaration
- **WHEN** the latest intake-ready Spec scope line is empty or does not identify an own change
- **THEN** the check fails with an error instead of accepting the task as legacy

#### Scenario: A renamed requirement is also modified in a delta
- **WHEN** an own change renames an existing requirement and modifies it under the new name
- **THEN** authorization to rename the original requirement does not authorize modifying it
- **AND** modification authorization is checked against the original requirement name

#### Scenario: A foreign change is rewritten while archived
- **WHEN** a foreign change is archived with altered file contents
- **THEN** both archive and edit authorization are required for that change

#### Scenario: Text is inserted into an existing requirement
- **WHEN** a pull request adds a sentence, scenario, or scenario step inside a requirement already present in main
- **THEN** matching modify authorization for that existing requirement is required
- **AND** adding a separate new requirement remains allowed without authorization

#### Scenario: An active foreign change is edited while copied to archive
- **WHEN** a foreign change is copied unchanged to archive while its active source is modified
- **THEN** both archive and edit authorization are required for that change

#### Scenario: A previous foreign archive is changed while another is created
- **WHEN** a pull request edits or removes an existing foreign archive and creates a new archive copy of the same change
- **THEN** archive authorization for the new copy does not authorize editing the previous archive

#### Scenario: Chained requirement renames retain their original authorization
- **WHEN** an own delta renames an existing requirement through intermediate names and modifies or removes it
- **THEN** the operation requires authorization for the original requirement in main
- **AND** cyclic or ambiguous rename mappings fail the check

#### Scenario: A new requirement takes an existing scenario
- **WHEN** an inserted requirement heading moves existing scenario lines out of a requirement that remains in the specification
- **THEN** the check requires modify authorization for that existing requirement

#### Scenario: Reusing a renamed requirement's former name
- **WHEN** an own delta renames an existing requirement and another requirement takes its former name
- **THEN** each rename remains authorized against its own original requirement in main
- **AND** modifying or removing the first requirement under its new name requires permission for that original requirement

#### Scenario: A requirement's text moves under another existing requirement
- **WHEN** a pull request deletes or moves the heading of an existing requirement so that some of its text or scenarios fall under another requirement already present in main
- **THEN** authorization to remove or modify the requirement whose heading changed does not cover the change
- **AND** modify authorization for the requirement that absorbs the text is required

#### Scenario: Scope declaration names its own change ambiguously
- **WHEN** the latest intake-ready Spec scope line names no change or more than one change inside its own(...) part, or names a change outside it
- **THEN** the check fails with an error instead of treating any other word as the own change

#### Scenario: An existing archive shares the own change name
- **WHEN** a pull request edits an archive that already exists in main under the same change name as the task's own change
- **THEN** the archive is treated as a foreign change and edit authorization is required

#### Scenario: A new capability differs from an existing one only by letter case or Unicode form
- **WHEN** a pull request adds a capability directory whose name equals an existing or another added capability after case folding and Unicode normalization
- **THEN** the check fails with an error instead of treating it as a new capability
- **AND** the same applies when a delta in the task's own change names a capability that differs from an existing one only by letter case or Unicode form

## REMOVED Requirements

### Requirement: A created release automatically back-merges main into dev
**Reason**: `dev` is gone; `main` is the only integration branch and already holds everything a release branch is cut from, so there is nothing to merge back.
**Migration**: none; a fix for a release lands in `main` first and is cherry-picked into `release/X.Y` (Releases are cut from release branches).

## RENAMED Requirements

- FROM: `### Requirement: Specification change authorization on dev`
- TO: `### Requirement: Specification change authorization on main`
