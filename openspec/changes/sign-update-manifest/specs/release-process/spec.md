## MODIFIED Requirements

### Requirement: The CDN layout is one archive per platform
Per platform, `scripts/release.sh` SHALL stage in `dist/cdn` the versioned `tilr-<version>-<platform>.tar.gz`, its `.sha256`, and `releases/<platform>/latest`. Public stable and beta SHALL use `latest` and `beta` respectively, with adjacent `<latest|beta>.manifest.json` authenticating version, platform, archive name and SHA-256. Both channels SHALL use the same versioned archives and local staging layout.

**Verified by**: itests `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`), whose CDN publishes this layout (`LocalCdn.publish`).

#### Scenario: One platform built
- **WHEN** `scripts/release.sh linux-x86_64` finishes for `0.1.0-26449dd`
- **THEN** its `.sha256` contains `<hex>  <archive>`, the local latest contains `<version>\n`, and the archive holds `tilrd` and `tilr` mode 0755, `LICENSE-APACHE`, `LICENSE-MIT` and `README.md` at its top level
- **AND** `dist/cdn/releases/0.1.0-26449dd/` holds `tilr-0.1.0-26449dd-linux-x86_64.tar.gz` and its `.sha256`, and `releases/linux-x86_64/latest` is `0.1.0-26449dd`

### Requirement: The CDN layout is a contract with the updater and the installer
The layout SHALL NOT change without `tilrd update` (`crates/tilrd/src/update.rs`) and the installer changing with it. Releases SHALL be published as the archive only, not also under the loose names of earlier releases (`tilrd-<platform>`, `tilr-server-<platform>`, `tilr-<platform>`). **BREAKING** (2a8b425): updaters from before the archive find no new release and are reinstalled once.

**Verified by**: `release::the_layout_release_sh_writes_is_what_tilrd_update_reads`.

#### Scenario: A release published
- **WHEN** a release is published for `linux-x86_64`
- **THEN** `releases/<version>/` holds only `tilr-<version>-linux-x86_64.tar.gz` and its `.sha256` for that platform, with signed metadata beside the selected channel pointer

### Requirement: Publishing uploads the archive before latest
With `--publish` and optional `--channel stable|beta` (default `stable`), `scripts/release.sh` SHALL upload each platform's archive and checksum before its signed manifest and selected pointer: `releases/<platform>/latest` for stable or `releases/<platform>/beta` for beta. Uploads SHALL use AWS CLI `aws s3 cp` to bucket `tilr-cdn` (`https://cdn.tilr.dev`) with the endpoint, metadata and success output specified below.

**Verified by**: `release::publishing_uploads_the_archive_and_its_checksum_before_latest`, `release::beta_publication_uploads_only_archives_checksums_and_selected_beta_pointers`, `release::beta_publication_failure_does_not_advance_the_pointer_or_publish_the_installer`.

#### Scenario: Publish two platforms
- **WHEN** `scripts/release.sh --publish linux-aarch64 linux-x86_64` runs with S3 credentials scoped to objects in `tilr-cdn`
- **THEN** each platform's archive and checksum are uploaded before its signed manifest and `latest`
- **AND** uploads use endpoint `https://<CLOUDFLARE_ACCOUNT_ID>.r2.cloudflarestorage.com`, region `auto`
- **AND** archives use `application/gzip`, checksums use `text/plain; charset=utf-8`, both with `public, max-age=31536000, immutable`; pointers use `text/plain; charset=utf-8` with `no-cache`
- **AND** stdout prints `published <version> for: <platforms>`

#### Scenario: Publish a candidate
- **WHEN** `scripts/release.sh --publish --channel beta linux-x86_64` runs
- **THEN** the archive and checksum are uploaded before the signed beta manifest and beta; latest and install.sh are untouched

### Requirement: --upload-only uploads what earlier builds laid out
`scripts/release.sh --upload-only <platforms>` SHALL build nothing; it SHALL check that every platform given has a `dist/cdn/releases/<platform>/latest`, that all name the same version and that each archive matches its `.sha256`, then sign all selected manifests before any upload and upload them as `--publish` does, with the same optional `--channel stable|beta`. Unselected platforms and channels SHALL remain unchanged.

**Verified by**: `release::upload_only_checks_what_was_built_and_leaves_other_platforms_alone`, `release::beta_publication_uploads_only_archives_checksums_and_selected_beta_pointers`.

#### Scenario: No staged platform
- **WHEN** a selected platform has no local latest file
- **THEN** upload-only fails with `nothing built for <platform>: no ...` before uploading

#### Scenario: Platforms built as different versions
- **WHEN** `linux-aarch64` was built as one version and `linux-x86_64` as another and `--upload-only` is given both
- **THEN** it says `<platform> is built as <v1>, not <v2>`, exits 1 and uploads nothing

#### Scenario: Upload a staged candidate
- **WHEN** a candidate is staged under local latest and `--upload-only --channel beta` runs for selected platforms
- **THEN** no build runs and only their archives, checksums, signed beta manifests and beta pointers are uploaded

### Requirement: The CDN gets the release last
After GitHub assets are attached, the `cdn` job SHALL verify each built platform carries `<tag version>-<commit>` and sign manifests and upload its completed `dist/cdn` layout with `scripts/release.sh --upload-only --channel <channel> <platforms>`, for both created releases and candidates. The channel SHALL be stable for a release and beta for a candidate. Both macOS platforms SHALL always be included; Linux arm64 SHALL remain optional.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** the `macos-aarch64`, `macos-x86_64` and `linux-x86_64` archives, checksums and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

#### Scenario: macOS publication
- **WHEN** both macOS builds complete and GitHub assets are attached
- **THEN** the CDN job uploads each macOS archive and checksum before that platform's selected channel pointer (latest for stable, beta for a candidate), without rebuilding or re-signing

#### Scenario: CDN credentials
- **WHEN** the CDN job publishes a release
- **THEN** it uses environment `cdn`, restricted to `release/*` branches, with bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets and the `CLOUDFLARE_ACCOUNT_ID` variable. The signing step SHALL use the `TILR_UPDATE_SIGNING_KEY` secret (Ed25519 PEM); build jobs SHALL embed the public `TILR_UPDATE_PUBLIC_KEY` repository variable and SHALL NOT receive the private key

#### Scenario: Candidate publication
- **WHEN** candidate GitHub assets have been attached
- **THEN** the CDN job uploads completed archives and checksums before signed beta manifests and beta for enabled platforms, without rebuilding or modifying latest or install.sh

