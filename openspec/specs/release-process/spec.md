# release-process Specification

## Purpose

Covers how a release is built, checked and published: the version string baked into each executable, the Linux build in a container, the release archive and the CDN layout `scripts/release.sh` produces and uploads, the CI gates every change passes, and the release-please / GitHub release / CDN flow (`.github/workflows/ci.yml`, `.github/workflows/release.yml`). How `tilrd update` and the installer consume the layout is the `update` and `installation` specs.

## Requirements

### Requirement: Every build names its version
Each executable SHALL carry a version `<package>-<commit>`: `<package>` is `TILR_BUILD_VERSION` or else `[workspace.package] version` (e.g. `0.1.0`); `<commit>` is `TILR_BUILD_COMMIT` if set and non-empty, else `git rev-parse --short HEAD`, else `dev`.

**Verified by**: `update::update_does_nothing_when_the_latest_release_is_the_one_installed` (it reads the version from `--version` and compares it with `latest`).

#### Scenario: Release build
- **WHEN** a release is built from commit `26449dd` of package version `0.1.0`
- **THEN** its version is `0.1.0-26449dd`

#### Scenario: Build without git
- **WHEN** a build runs where git cannot name the commit and `TILR_BUILD_COMMIT` is unset
- **THEN** the version is `0.1.0-dev`

### Requirement: --version prints the version and the platform
`tilrd --version` SHALL print `tilrd <version> (<platform>)` and `tilr --version` `tilr <version> (<platform>)`, the platform being `<os>-<arch>` of the target as releases name it (`linux-aarch64`, `linux-x86_64`, `macos-aarch64`). `tilr version` SHALL print the client's line and the running server's version.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: Linux arm64 build
- **WHEN** `tilrd --version` runs on a release built for linux aarch64 from `26449dd`
- **THEN** it prints `tilrd 0.1.0-26449dd (linux-aarch64)`

### Requirement: Linux binaries are built in a Debian 12 container
`scripts/linux-build.sh` SHALL run `cargo build --release -p tilrd -p tilr` (no `failpoints`) through `scripts/linux.sh` in the image of `scripts/linux.Dockerfile` (`rust:1.99-bookworm` pinned by digest: glibc 2.36, `protobuf-compiler`, Zig 0.15.2 from `scripts/install-tools.sh`) for the architecture `TILR_LINUX_PLATFORM` names (default the host's), passing `TILR_BUILD_COMMIT` in, and copy the results to `dist/linux-<arch>/tilrd` and `dist/linux-<arch>/tilr`.

**Verified by**: `release::the_linux_build_runs_in_the_pinned_debian_12_image` (static: `scripts/linux.Dockerfile` and `scripts/linux-build.sh`).

#### Scenario: amd64 from another machine
- **WHEN** `TILR_LINUX_PLATFORM=linux/amd64 scripts/linux-build.sh` runs
- **THEN** `dist/linux-x86_64/tilrd` and `dist/linux-x86_64/tilr` are written

### Requirement: A release is built from a clean commit
`scripts/release.sh` SHALL refuse to build when `git status --porcelain` is not empty (`uncommitted changes: a release is built from a commit`, exit 1). The version SHALL be the root `Cargo.toml`'s first `version = "..."`, `-`, and the short commit, passed to the build as `TILR_BUILD_COMMIT` so the binaries name it.

**Verified by**: `release::a_dirty_tree_builds_no_release` and `release::a_release_is_named_after_the_package_version_and_the_commit`.

#### Scenario: Dirty tree
- **WHEN** `scripts/release.sh` runs with an uncommitted change
- **THEN** it prints `uncommitted changes: a release is built from a commit` and exits 1 without building

### Requirement: Platforms are built per argument
`scripts/release.sh [--publish] [PLATFORM...]` SHALL build each platform given, `linux-aarch64` (Docker `linux/arm64`) or `linux-x86_64` (`linux/amd64`), default `linux-aarch64`; another name SHALL fail with `unknown platform: <name>` and exit 2. Each platform SHALL get its own `latest`, so a release can be out for one platform and not yet another.

**Verified by**: `release::each_platform_given_is_built_with_its_own_latest` and `release::an_unknown_platform_is_refused`.

#### Scenario: Unknown platform
- **WHEN** `scripts/release.sh linux-riscv64` runs
- **THEN** it prints `unknown platform: linux-riscv64` and exits 2

### Requirement: The CDN layout is one archive per platform
Per platform, `scripts/release.sh` SHALL lay out in `dist/cdn`: `releases/<version>/tilr-<version>-<platform>.tar.gz` (gzip tar holding `tilrd` and `tilr`, mode 0755, with `LICENSE-APACHE`, `LICENSE-MIT` and `README.md`, all at its top level), its `.sha256` (`<hex>  <archive>`), and `releases/<platform>/latest` holding `<version>\n`.

**Verified by**: itests `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`), whose CDN publishes this layout (`LocalCdn.publish`).

#### Scenario: One platform built
- **WHEN** `scripts/release.sh linux-x86_64` finishes for `0.1.0-26449dd`
- **THEN** `dist/cdn/releases/0.1.0-26449dd/` holds `tilr-0.1.0-26449dd-linux-x86_64.tar.gz` and its `.sha256`, and `releases/linux-x86_64/latest` is `0.1.0-26449dd`

### Requirement: The CDN layout is a contract with the updater and the installer
The layout SHALL NOT change without `tilrd update` (`crates/tilrd/src/update.rs`) and the installer changing with it. Releases SHALL be published as the archive only, not also under the loose names of earlier releases (`tilrd-<platform>`, `tilr-server-<platform>`, `tilr-<platform>`). **BREAKING** (2a8b425): updaters from before the archive find no new release and are reinstalled once.

**Verified by**: `release::the_layout_release_sh_writes_is_what_tilrd_update_reads`.

#### Scenario: A release published
- **WHEN** a release is published for `linux-x86_64`
- **THEN** `releases/<version>/` holds only `tilr-<version>-linux-x86_64.tar.gz` and its `.sha256` for that platform

### Requirement: Publishing uploads the archive before latest
With `--publish`, `scripts/release.sh` SHALL upload with `wrangler r2 object put ... --remote` to bucket `tilr-cdn` (`https://cdn.tilr.dev`), per platform: the archive (`application/gzip`) and its `.sha256` (`text/plain; charset=utf-8`) with `public, max-age=31536000, immutable`, and only then `releases/<platform>/latest` with `no-cache`, printing `published <version> for: <platforms>`.

**Verified by**: `release::publishing_uploads_the_archive_and_its_checksum_before_latest`.

#### Scenario: Publish two platforms
- **WHEN** `scripts/release.sh --publish linux-aarch64 linux-x86_64` runs with wrangler credentials
- **THEN** each platform's archive and checksum are uploaded before its `latest`

### Requirement: Uploading needs the bucket's account id
`scripts/release.sh --publish` and `--upload-only` SHALL fail with `CLOUDFLARE_ACCOUNT_ID is not set: export the account id of the tilr-cdn bucket ...` and exit 2 when `CLOUDFLARE_ACCOUNT_ID` is not exported, before building or uploading anything.

**Verified by**: `release::publishing_without_the_account_id_does_nothing`.

#### Scenario: No account id
- **WHEN** `scripts/release.sh --publish` runs with `CLOUDFLARE_ACCOUNT_ID` unset
- **THEN** it says `CLOUDFLARE_ACCOUNT_ID is not set` and exits 2 before building or uploading

### Requirement: --upload-only uploads what earlier builds laid out
`scripts/release.sh --upload-only <platforms>` SHALL build nothing; it SHALL check that every platform given has a `dist/cdn/releases/<platform>/latest` (`nothing built for <platform>: no ...` otherwise), that all name the same version and that each archive matches its `.sha256`, then upload them as `--publish` does. The `latest` of a platform not given SHALL be left as it is.

**Verified by**: `release::upload_only_checks_what_was_built_and_leaves_other_platforms_alone`.

#### Scenario: Platforms built as different versions
- **WHEN** `linux-aarch64` was built as one version and `linux-x86_64` as another and `--upload-only` is given both
- **THEN** it says `<platform> is built as <v1>, not <v2>`, exits 1 and uploads nothing

### Requirement: CI gates every change
Every pull request and push to `main` SHALL pass `.github/workflows/ci.yml`: `cargo fmt --all --check`; clippy `--workspace --all-targets --locked -- -D warnings`, with and without `--features tilrd/failpoints`; `cargo test --workspace --features tilrd/failpoints --locked` on Linux and macOS 15; `cargo check` with failpoints at the MSRV 1.90; `cargo deny check`; `gitleaks git` over the whole history.

**Verified by**: `release::ci_runs_every_gate_of_agents_md` (static: `.github/workflows/ci.yml`).

#### Scenario: A lint finding
- **WHEN** a pull request has a clippy warning
- **THEN** the `fmt + clippy` job fails and the change is not merged

### Requirement: CI smoke-tests the built executables
CI SHALL build `tilrd` and `tilr` in debug and check, for each, that `--version` is `<name> <package version>-<short commit> (<platform>)` and that `--help` runs.

**Verified by**: `release::ci_smoke_tests_both_executables` (static: `.github/workflows/ci.yml`).

#### Scenario: Wrong version string
- **WHEN** the built `--version` of either does not name the package version and the commit
- **THEN** the `test` job fails

### Requirement: Build tools are pinned and checked
`scripts/install-tools.sh DIR [zig|protoc]` SHALL install Zig 0.15.2 and protoc 36.2 into `DIR` for Linux or macOS on x86_64 or aarch64, each checked against a pinned SHA-256; CI, release jobs and the Linux build image SHALL use it.

**Verified by**: `release::build_tools_are_pinned_by_checksum_and_used_everywhere` (static: `scripts/install-tools.sh`, the workflows and `scripts/linux.Dockerfile`).

#### Scenario: Install both
- **WHEN** `scripts/install-tools.sh "$RUNNER_TEMP/tools"` runs on Linux x86_64
- **THEN** `DIR/zig` and `DIR/protoc` are in place after their checksums matched

### Requirement: release-please opens the release PR
On `main`, release-please (`simple` strategy, `release-please-config.json`, acting as the `tilr-bot` GitHub App) SHALL keep a release PR open from the `feat` / `fix` Conventional Commits, bumping `[workspace.package] version` in `Cargo.toml`, the workspace crates' versions in `Cargo.lock`, and `CHANGELOG.md`. Versions and the changelog SHALL NOT be edited by hand. The release PR SHALL pass the same required checks as any PR.

**Verified by**: `release::release_please_bumps_the_workspace_version_and_lock` (static: `release-please-config.json`, `.github/workflows/release.yml`).

#### Scenario: A feat commit lands on main
- **WHEN** a `feat:` commit is pushed to `main`
- **THEN** the release PR is opened or updated with the next minor version (`bump-patch-for-minor-pre-major` is false)

### Requirement: Merging the release PR builds and attaches the binaries
Merging the release PR SHALL tag `vX.Y.Z` and create the GitHub release; `tilrd` and `tilr` SHALL be built per target (`aarch64-apple-darwin`, `x86_64-apple-darwin` cross-compiled, `x86_64-unknown-linux-gnu` by `scripts/release.sh`, `aarch64-unknown-linux-gnu` only with repository variable `RELEASE_LINUX_ARM64=true`), native builds' `--version` checked against tag and commit, and attached by `scripts/package.sh` as `tilr-<tag>-<target>.tar.gz` (`tilrd`, `tilr`, licenses, README) with `.sha256`.

**Verified by**: `release::a_release_builds_and_packages_every_target` (static: `.github/workflows/release.yml`, `scripts/package.sh`).

#### Scenario: Release v0.2.0
- **WHEN** the release PR for `0.2.0` is merged
- **THEN** tag `v0.2.0` has a GitHub release with one `tilr-v0.2.0-<target>.tar.gz` and checksum per target, each binary saying `0.2.0-<commit>`

### Requirement: The CDN gets the release last
After the GitHub release has its assets, the `cdn` job (environment `cdn`, restricted to `main`, holding `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`) SHALL check every Linux platform built carries `<tag version>-<commit>` and run `scripts/release.sh --upload-only <platforms>` on the `dist/cdn` the Linux builds laid out; with arm64 off only `linux-x86_64` is uploaded and `releases/linux-aarch64/latest` is left as it is.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** only the `linux-x86_64` archive, checksum and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

### Requirement: Pinned Zig uses a compatible macOS SDK in clean worktrees
While the build tools use Zig 0.15.2, the repository SHALL select the Command Line Tools macOS 15.4 SDK by default for Cargo builds on macOS, including clean worktrees without local Cargo configuration. A caller MAY select another installed macOS 15.4 SDK with `DEVELOPER_DIR` and `SDKROOT`. The Linux build image SHALL continue to build without a macOS SDK.

#### Scenario: Direct clean macOS build
- **WHEN** a fresh worktree on a Mac with the Command Line Tools macOS 15.4 SDK runs `cargo build --workspace --features tilrd/failpoints` with an empty target directory and the default SDK is macOS 27.0
- **THEN** `libghostty-vt-sys` 0.2.2 builds with Zig 0.15.2 using the selected macOS 15.4 SDK and the workspace build succeeds

#### Scenario: CI uses Xcode's macOS 15.4 SDK
- **WHEN** a macOS CI or release runner has macOS 15.4 SDK in Xcode 16.3 rather than Command Line Tools
- **THEN** the workflow selects that Xcode and SDK for the installer and Cargo build

#### Scenario: Linux build image
- **WHEN** `scripts/linux.sh cargo build --workspace --features tilrd/failpoints --locked` runs
- **THEN** the build succeeds without a macOS SDK

**Verified by**: `release::the_pinned_zig_selects_a_compatible_macos_sdk` (static: Cargo defaults and workflows), `scripts/test-macos-sdk.sh`, a fresh macOS workspace build, and a fresh Linux workspace build via `scripts/linux.sh`.

### Requirement: macOS Zig installer checks its SDK
`scripts/install-tools.sh DIR zig` on macOS SHALL verify the selected macOS 15.4 SDK before downloading Zig and SHALL fail with setup guidance when it is absent.

#### Scenario: Missing compatible SDK during tool installation
- **WHEN** `scripts/install-tools.sh DIR zig` runs on macOS without the selected macOS 15.4 SDK
- **THEN** it fails before downloading Zig and explains how to install or select the required SDK

**Verified by**: `release::the_macos_zig_installer_checks_its_sdk_before_download` (runs the mocked installer preflight in `scripts/test-macos-sdk.sh`).

### Requirement: Linux worktrees isolate build outputs
`scripts/linux.sh` SHALL mount a persistent cargo target volume specific to the physical worktree path and platform, named `tilr-linux-target-<path hash><platform suffix>`. The cargo registry SHALL remain shared per platform. `scripts/linux-build.sh` SHALL build and copy binaries from the same selected target.

**Verified by:** `linux_script::parallel_worktrees_keep_their_linux_build_outputs` and `scripts/test-linux.py --docker`.

#### Scenario: Overlapping builds
- **WHEN** worktree A builds its binary and waits, and worktree B (even with an identical directory basename) builds a different binary before A resumes
- **THEN** both commands succeed and A still runs its own binary

#### Scenario: Reuse and cleanup
- **WHEN** the same physical checkout is invoked again, including through a symlink
- **THEN** it uses the same target, which can be removed with `docker volume rm <volume name>` after its containers stop

#### Scenario: Volume naming and platform separation
- **WHEN** a checkout runs a command for a selected `TILR_LINUX_PLATFORM`
- **THEN** the path hash is Git's hash-object of its physical absolute checkout path followed by a newline
- **AND** the platform suffix replaces each non-lowercase-alphanumeric character of `TILR_LINUX_PLATFORM` with `-` (empty when unset)
- **AND** the same platform uses the same shared registry while different platforms use separate target and registry volumes

#### Scenario: Linux release copy
- **WHEN** `scripts/linux-build.sh` builds release binaries in a worktree
- **THEN** the copied `dist/linux-<arch>/tilrd` and `tilr` come from that worktree's target

### Requirement: Linux commands run their built image
`scripts/linux.sh` SHALL run the immutable image ID returned by its own successful build, even when another invocation rebuilds the shared image tag. A failed image build SHALL fail the invocation before running a container.

**Verified by:** `linux_script::parallel_worktrees_keep_their_linux_build_outputs`.

#### Scenario: Another worktree rebuilds the tag
- **WHEN** another worktree updates the platform's image tag between a build and its container start
- **THEN** the command still runs the image produced by its own build

#### Scenario: Failed build
- **WHEN** Docker fails to build the image
- **THEN** the script exits unsuccessfully without starting its command

### Requirement: Outgoing WebSockets use patched bundled roots
tilrd, tilr and tilr-driver SHALL verify outgoing secure WebSocket certificates and hostnames with the bundled Mozilla roots from webpki-roots >=1.0.10 and rustls-webpki >=0.103.16, without webpki-roots 0.26, native trust-store substitution or advisory ignores. Cloud control, `tilrd ps`, `tilr --connect` and tilr-driver SHALL use the same explicit rustls connector; plain WebSockets SHALL remain supported.

**Verified by**: unit tests in `crates/tilr-websocket/src/lib.rs`, `crates/tilr/src/conn.rs` and `crates/tilr-driver/src/session.rs`, and `cargo deny check`.

#### Scenario: Secure WebSocket peer
- **WHEN** Cloud control, `tilrd ps`, `tilr --connect` or tilr-driver connects to a secure WebSocket
- **THEN** it verifies the peer against the bundled roots and rejects an untrusted issuer or a hostname mismatch

#### Scenario: Plain WebSocket peer
- **WHEN** tilrd, tilr or tilr-driver connects to a plain WebSocket
- **THEN** it performs the WebSocket handshake without TLS and preserves request headers

#### Scenario: Dependency regression
- **WHEN** the dependency graph contains an affected roots or verifier release
- **THEN** the regression check or cargo deny fails instead of ignoring the advisory

### Requirement: The installer is published after the release it installs
`scripts/release.sh --publish` and `--upload-only` SHALL upload `scripts/install.sh` to `install.sh` in bucket `tilr-cdn` (`text/x-shellscript; charset=utf-8`, `public, max-age=300`) after the `latest` of every platform given, and SHALL NOT upload it when any of those uploads failed. The served installer SHALL be the one the itests run (`scripts/install.sh`).

**Verified by**: `release::the_installer_is_uploaded_after_latest` (a test of `scripts/release.sh --upload-only` with a stub `wrangler` on `PATH` that logs its calls).

#### Scenario: Upload order
- **WHEN** `scripts/release.sh --upload-only linux-x86_64` runs with a stub `wrangler` that logs each `r2 object put`
- **THEN** the log names the archive, its `.sha256`, `releases/linux-x86_64/latest` and then `install.sh`, in that order

#### Scenario: A failed upload
- **WHEN** the stub `wrangler` fails on `releases/linux-x86_64/latest`
- **THEN** the script exits non-zero and `install.sh` is not uploaded

### Requirement: Shared Ghostty VT development build

The repository SHALL provide a script that builds the static Ghostty VT library pinned by libghostty-vt-sys, using its ReleaseFast, baseline CPU and no application runtime settings. The script SHALL select the same default macOS SDK as Cargo, and SHALL identify the shared prefix by Ghostty revision, Zig version, platform and SDK. It SHALL publish a libghostty-vt-static pkg-config file only with a completed build and SHALL reuse an already completed prefix without cloning or building again.

#### Scenario: Build and reuse a shared prefix
- **WHEN** a developer runs the shared build script with a supported Zig installation
- **THEN** it prints a prefix containing the static library and share/pkgconfig/libghostty-vt-static.pc
- **AND** a subsequent invocation with the same inputs reuses that prefix without cloning Ghostty or invoking zig build

#### Scenario: Toolchain identity changes
- **WHEN** the Ghostty revision, Zig version, platform or selected SDK changes
- **THEN** the script selects a distinct prefix

#### Scenario: Failed build is retried explicitly
- **WHEN** a shared build fails before completing
- **THEN** no completed prefix is published and a later invocation can build it again

### Requirement: Cargo discovers the shared Ghostty VT library

The tilr-screen dependency graph SHALL enable libghostty-vt-sys pkg-config discovery. A matching library supplied through PKG_CONFIG_PATH SHALL replace the per-target source build; when no package is discoverable, the existing source build SHALL remain available. Developer documentation SHALL explain setting PKG_CONFIG_PATH and rebuilding after a dependency or toolchain upgrade.

#### Scenario: Clean Cargo build with shared library
- **WHEN** a clean cargo build --tests runs with PKG_CONFIG_PATH pointing to the shared package
- **THEN** libghostty-vt-sys links that static library without cloning Ghostty or invoking zig build

#### Scenario: Cargo source fallback
- **WHEN** cargo build --tests runs without a discoverable shared package
- **THEN** libghostty-vt-sys builds the pinned Ghostty source as before

### Requirement: Interrupted shared Ghostty build recovery

The shared build script SHALL record its owner PID in the prefix lock. When that process is no longer alive, a subsequent invocation SHALL recover the lock, remove abandoned temporary build directories for that prefix, and build again. It SHALL refuse to disturb a lock owned by a live process. Before publishing a completed installation, it SHALL remove an incomplete destination prefix.

Recovery SHALL be serialized by an advisory lock that is released on process exit, including an untrappable termination. A lock directory left before its owner PID was recorded SHALL also be recoverable.

#### Scenario: Recover an abandoned build
- **WHEN** a lock records the PID of an exited process and temporary build directories remain
- **THEN** the next invocation builds and publishes the completed prefix
- **AND** no lock or temporary build directories remain

#### Scenario: Preserve an active build
- **WHEN** the lock records a live process PID
- **THEN** the invocation reports that a build is in progress without removing its lock or temporary directories

#### Scenario: Replace an incomplete prefix
- **WHEN** the destination exists without a completion marker
- **THEN** a successful build replaces it with the library and pkg-config file at the expected paths

#### Scenario: Interrupted owner initialization
- **WHEN** a previous build left a lock directory without a PID and no process holds the advisory lock
- **THEN** the next invocation recovers the directory and publishes a completed prefix

#### Scenario: Concurrent recovery
- **WHEN** one invocation holds the advisory lock while recovering an abandoned build
- **THEN** another invocation reports a build in progress without removing the first invocation's lock or working directory, even if the owner metadata is stale

#### Scenario: Interrupted cleanup after publication
- **WHEN** a completed prefix exists alongside an abandoned lock or temporary build directory
- **THEN** the next invocation removes the abandoned state under the advisory lock and reuses the prefix without fetching or building again

## Notes

- Release scripts and workflow contracts are covered by `crates/tilrd/tests/release.rs`; workflow checks inspect source text and do not execute GitHub Actions.
- CI, release-please (`release-please-config.json`, `.release-please-manifest.json`), `deny.toml`, `scripts/package.sh` and `scripts/install-tools.sh` came to this branch with the merge of `main` (47a8557), ported to the `tilrd` / `tilr` names; the archive layout is 2a8b425.
- The `failpoints` feature is never enabled by the release scripts; a release has none of the fault injection points.
