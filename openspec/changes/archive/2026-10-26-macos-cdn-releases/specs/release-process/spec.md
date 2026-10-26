## MODIFIED Requirements

### Requirement: --version prints the version and the platform
`tilrd --version` SHALL print `tilrd <version> (<platform>)` and `tilr --version` `tilr <version> (<platform>)`, the platform being `<os>-<arch>` of the target as releases name it (`linux-aarch64`, `linux-x86_64`, `macos-aarch64`, `macos-x86_64`). `tilr version` SHALL print the client's line and the running server's version.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: Linux arm64 build
- **WHEN** `tilrd --version` runs on a release built for linux aarch64 from `26449dd`
- **THEN** it prints `tilrd 0.1.0-26449dd (linux-aarch64)`

#### Scenario: Intel macOS build
- **WHEN** `tilrd --version` runs on a release built for x86_64 macOS
- **THEN** its platform is `macos-x86_64`

### Requirement: Platforms are built per argument
`scripts/release.sh [--publish] [PLATFORM...]` SHALL build each platform given, `linux-aarch64` (Docker `linux/arm64`) `linux-x86_64` (`linux/amd64`), `macos-aarch64` (`aarch64-apple-darwin`) or `macos-x86_64` (`x86_64-apple-darwin`), default `linux-aarch64`; another name SHALL fail with `unknown platform: <name>` and exit 2. Each platform SHALL get its own `latest`, so a release can be out for one platform and not yet another.

**Verified by**: `release::each_platform_given_is_built_with_its_own_latest` and `release::an_unknown_platform_is_refused`.

#### Scenario: Unknown platform
- **WHEN** `scripts/release.sh linux-riscv64` runs
- **THEN** it prints `unknown platform: linux-riscv64` and exits 2

#### Scenario: Both macOS architectures
- **WHEN** `scripts/release.sh macos-aarch64 macos-x86_64` runs on macOS with signing and notarization configured
- **THEN** it builds both Apple targets and produces their separate CDN archives, checksums and latest pointers

### Requirement: Merging the release PR builds and attaches the binaries
Merging the release PR SHALL tag `vX.Y.Z` and create a GitHub release with `tilrd` and `tilr` archives and checksums for every enabled target. Native builds SHALL have their version checked against the tag and commit. macOS builds SHALL be Developer ID signed and accepted by Apple notarization before packaging.

**Verified by**: `release::a_release_builds_and_packages_every_target` (static: `.github/workflows/release.yml`, `scripts/package.sh`).

#### Scenario: Release v0.2.0
- **WHEN** the release PR for `0.2.0` is merged
- **THEN** tag `v0.2.0` has a GitHub release with one `tilr-v0.2.0-<target>.tar.gz` and checksum per target, each binary saying `0.2.0-<commit>`

#### Scenario: Apple rejects a build
- **WHEN** signing, signature verification or notarization fails for either macOS target
- **THEN** the build job fails and neither GitHub assets nor CDN latest pointers are published by the release workflow

#### Scenario: Release target selection and packaging
- **WHEN** the release build matrix runs
- **THEN** it builds `aarch64-apple-darwin` and cross-compiles `x86_64-apple-darwin` on macOS, and builds `x86_64-unknown-linux-gnu` plus `aarch64-unknown-linux-gnu` only with `RELEASE_LINUX_ARM64=true`
- **AND** every platform is built through `scripts/release.sh`
- **AND** `scripts/package.sh` attaches `tilr-<tag>-<target>.tar.gz` containing `tilrd`, `tilr`, licenses and README with its `.sha256`

### Requirement: The CDN gets the release last
After GitHub assets are attached, the `cdn` job SHALL verify each built platform carries `<tag version>-<commit>` and upload its completed `dist/cdn` layout with `scripts/release.sh --upload-only <platforms>`. Both macOS platforms SHALL always be included; Linux arm64 SHALL remain optional.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** the `macos-aarch64`, `macos-x86_64` and `linux-x86_64` archives, checksums and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

#### Scenario: macOS publication
- **WHEN** both macOS builds complete and GitHub assets are attached
- **THEN** the CDN job uploads each macOS archive and checksum before that platform's latest, without rebuilding or re-signing

#### Scenario: CDN credentials
- **WHEN** the CDN job publishes a release
- **THEN** it uses environment `cdn`, restricted to `main`, with bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets and the `CLOUDFLARE_ACCOUNT_ID` variable

## ADDED Requirements

### Requirement: macOS releases require Developer ID and accepted notarization
The macOS release helper SHALL require macOS and Apple signing credentials, sign and verify both executables with Developer ID, hardened runtime and a secure timestamp, and require Accepted notarization before producing distributable artifacts. CI SHALL use a temporary keychain and remove it even on failure.

#### Scenario: Signed bytes are published
- **WHEN** both macOS executables are signed and notarization is Accepted
- **THEN** the CDN and GitHub archives contain those exact signed executable bytes and checksums describe the completed archives

#### Scenario: Missing Apple credentials
- **WHEN** a macOS build lacks its identity or notary profile
- **THEN** it fails before building or publishing

#### Scenario: Upload previously completed macOS builds
- **WHEN** upload-only receives completed macOS CDN layouts
- **THEN** it checks their versions and checksums and uploads them without requiring Apple credentials

#### Scenario: Signing and notarization configuration
- **WHEN** a macOS release is built
- **THEN** a Developer ID Application identity and a notarytool keychain profile are required before building
- **AND** both signed executables are submitted together in a ZIP to Apple

#### Scenario: Signing or notarization fails
- **WHEN** signing or signature verification fails, or notarization fails, returns malformed output or a status other than Accepted
- **THEN** no distributable archive or new latest pointer is produced or published

#### Scenario: CI imports Apple credentials
- **WHEN** the macOS build job starts
- **THEN** it imports the certificate and notary credentials from the cdn environment into a temporary keychain
- **AND** that keychain is removed even on failure
