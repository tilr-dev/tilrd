## MODIFIED Requirements

### Requirement: Linux binaries are built in a Debian 12 container
`scripts/linux-build.sh` SHALL run `cargo build --release -p tilrd -p tilr` (no `failpoints`) through `scripts/linux.sh` in the image of `scripts/linux.Dockerfile` (`rust:1.99-bookworm` pinned by digest: glibc 2.36, `protobuf-compiler`, Zig 0.15.2 from `scripts/install-tools.sh`) for the architecture `TILR_LINUX_PLATFORM` names (default the host's), passing `TILR_BUILD_COMMIT` in, and copy the results to `dist/linux-<arch>/tilrd` and `dist/linux-<arch>/tilr`.

**Verified by**: `release::the_linux_build_runs_in_the_pinned_debian_12_image` (static: `scripts/linux.Dockerfile` and `scripts/linux-build.sh`; planned in change release-process-tests).

#### Scenario: amd64 from another machine
- **WHEN** `TILR_LINUX_PLATFORM=linux/amd64 scripts/linux-build.sh` runs
- **THEN** `dist/linux-x86_64/tilrd` and `dist/linux-x86_64/tilr` are written

### Requirement: A release is built from a clean commit
`scripts/release.sh` SHALL refuse to build when `git status --porcelain` is not empty (`uncommitted changes: a release is built from a commit`, exit 1). The version SHALL be the root `Cargo.toml`'s first `version = "..."`, `-`, and the short commit, passed to the build as `TILR_BUILD_COMMIT` so the binaries name it.

**Verified by**: `release::a_dirty_tree_builds_no_release` and `release::a_release_is_named_after_the_package_version_and_the_commit` (planned in change release-process-tests).

#### Scenario: Dirty tree
- **WHEN** `scripts/release.sh` runs with an uncommitted change
- **THEN** it prints `uncommitted changes: a release is built from a commit` and exits 1 without building

### Requirement: Platforms are built per argument
`scripts/release.sh [--publish] [PLATFORM...]` SHALL build each platform given, `linux-aarch64` (Docker `linux/arm64`) or `linux-x86_64` (`linux/amd64`), default `linux-aarch64`; another name SHALL fail with `unknown platform: <name>` and exit 2. Each platform SHALL get its own `latest`, so a release can be out for one platform and not yet another.

**Verified by**: `release::each_platform_given_is_built_with_its_own_latest` and `release::an_unknown_platform_is_refused` (planned in change release-process-tests).

#### Scenario: Unknown platform
- **WHEN** `scripts/release.sh linux-riscv64` runs
- **THEN** it prints `unknown platform: linux-riscv64` and exits 2

### Requirement: The CDN layout is a contract with the updater and the installer
The layout SHALL NOT change without `tilrd update` (`crates/tilrd/src/update.rs`) and the installer changing with it. Releases SHALL be published as the archive only, not also under the loose names of earlier releases (`tilrd-<platform>`, `tilr-server-<platform>`, `tilr-<platform>`). **BREAKING** (2a8b425): updaters from before the archive find no new release and are reinstalled once.

**Verified by**: `release::the_layout_release_sh_writes_is_what_tilrd_update_reads` (planned in change release-process-tests).

#### Scenario: A release published
- **WHEN** a release is published for `linux-x86_64`
- **THEN** `releases/<version>/` holds only `tilr-<version>-linux-x86_64.tar.gz` and its `.sha256` for that platform

### Requirement: Publishing uploads the archive before latest
With `--publish`, `scripts/release.sh` SHALL upload with `wrangler r2 object put ... --remote` to bucket `tilr-cdn` (`https://cdn.tilr.dev`), per platform: the archive (`application/gzip`) and its `.sha256` (`text/plain; charset=utf-8`) with `public, max-age=31536000, immutable`, and only then `releases/<platform>/latest` with `no-cache`, printing `published <version> for: <platforms>`.

**Verified by**: `release::publishing_uploads_the_archive_and_its_checksum_before_latest` (planned in change release-process-tests).

#### Scenario: Publish two platforms
- **WHEN** `scripts/release.sh --publish linux-aarch64 linux-x86_64` runs with wrangler credentials
- **THEN** each platform's archive and checksum are uploaded before its `latest`

### Requirement: Uploading needs the bucket's account id
`scripts/release.sh --publish` and `--upload-only` SHALL fail with `CLOUDFLARE_ACCOUNT_ID is not set: export the account id of the tilr-cdn bucket ...` and exit 2 when `CLOUDFLARE_ACCOUNT_ID` is not exported, before building or uploading anything.

**Verified by**: `release::publishing_without_the_account_id_does_nothing` (planned in change release-process-tests).

#### Scenario: No account id
- **WHEN** `scripts/release.sh --publish` runs with `CLOUDFLARE_ACCOUNT_ID` unset
- **THEN** it says `CLOUDFLARE_ACCOUNT_ID is not set` and exits 2 before building or uploading

### Requirement: --upload-only uploads what earlier builds laid out
`scripts/release.sh --upload-only <platforms>` SHALL build nothing; it SHALL check that every platform given has a `dist/cdn/releases/<platform>/latest` (`nothing built for <platform>: no ...` otherwise), that all name the same version and that each archive matches its `.sha256`, then upload them as `--publish` does. The `latest` of a platform not given SHALL be left as it is.

**Verified by**: `release::upload_only_checks_what_was_built_and_leaves_other_platforms_alone` (planned in change release-process-tests).

#### Scenario: Platforms built as different versions
- **WHEN** `linux-aarch64` was built as one version and `linux-x86_64` as another and `--upload-only` is given both
- **THEN** it says `<platform> is built as <v1>, not <v2>`, exits 1 and uploads nothing

### Requirement: CI gates every change
Every pull request and push to `main` SHALL pass `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo test --workspace --features tilrd/failpoints --locked` on Linux and macOS 15; `cargo check` with failpoints at the MSRV 1.90; `cargo deny check`; `gitleaks git` over the whole history.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`; planned in change release-process-tests).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

### Requirement: CI smoke-tests the built executables
CI SHALL build `tilrd` and `tilr` in debug and check, for each, that `--version` is `<name> <package version>-<short commit> (<platform>)` and that `--help` runs.

**Verified by**: `release::ci_smoke_tests_both_executables` (static: `.github/workflows/ci.yml`; planned in change release-process-tests).

#### Scenario: Wrong version string
- **WHEN** the built `--version` of either does not name the package version and the commit
- **THEN** the `test` job fails

### Requirement: Build tools are pinned and checked
`scripts/install-tools.sh DIR [zig|protoc]` SHALL install Zig 0.15.2 and protoc 36.2 into `DIR` for Linux or macOS on x86_64 or aarch64, each checked against a pinned SHA-256; CI, release jobs and the Linux build image SHALL use it.

**Verified by**: `release::build_tools_are_pinned_by_checksum_and_used_everywhere` (static: `scripts/install-tools.sh`, the workflows and `scripts/linux.Dockerfile`; planned in change release-process-tests).

#### Scenario: Install both
- **WHEN** `scripts/install-tools.sh "$RUNNER_TEMP/tools"` runs on Linux x86_64
- **THEN** `DIR/zig` and `DIR/protoc` are in place after their checksums matched

### Requirement: release-please opens the release PR
On `main`, release-please (`simple` strategy, `release-please-config.json`, acting as the `tilr-bot` GitHub App) SHALL keep a release PR open from the `feat` / `fix` Conventional Commits, bumping `[workspace.package] version` in `Cargo.toml`, the workspace crates' versions in `Cargo.lock`, and `CHANGELOG.md`. Versions and the changelog SHALL NOT be edited by hand. The release PR SHALL pass the same required checks as any PR.

**Verified by**: `release::release_please_bumps_the_workspace_version_and_lock` (static: `release-please-config.json`, `.github/workflows/release.yml`; planned in change release-process-tests).

#### Scenario: A feat commit lands on main
- **WHEN** a `feat:` commit is pushed to `main`
- **THEN** the release PR is opened or updated with the next minor version (`bump-patch-for-minor-pre-major` is false)

### Requirement: Merging the release PR builds and attaches the binaries
Merging the release PR SHALL tag `vX.Y.Z` and create the GitHub release; `tilrd` and `tilr` SHALL be built per target (`aarch64-apple-darwin`, `x86_64-apple-darwin` cross-compiled, `x86_64-unknown-linux-gnu` by `scripts/release.sh`, `aarch64-unknown-linux-gnu` only with repository variable `RELEASE_LINUX_ARM64=true`), native builds' `--version` checked against tag and commit, and attached by `scripts/package.sh` as `tilr-<tag>-<target>.tar.gz` (`tilrd`, `tilr`, licenses, README) with `.sha256`.

**Verified by**: `release::a_release_builds_and_packages_every_target` (static: `.github/workflows/release.yml`, `scripts/package.sh`; planned in change release-process-tests).

#### Scenario: Release v0.2.0
- **WHEN** the release PR for `0.2.0` is merged
- **THEN** tag `v0.2.0` has a GitHub release with one `tilr-v0.2.0-<target>.tar.gz` and checksum per target, each binary saying `0.2.0-<commit>`

### Requirement: The CDN gets the release last
After the GitHub release has its assets, the `cdn` job (environment `cdn`, restricted to `main`, holding `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`) SHALL check every Linux platform built carries `<tag version>-<commit>` and run `scripts/release.sh --upload-only <platforms>` on the `dist/cdn` the Linux builds laid out; with arm64 off only `linux-x86_64` is uploaded and `releases/linux-aarch64/latest` is left as it is.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`; planned in change release-process-tests).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** only the `linux-x86_64` archive, checksum and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged
