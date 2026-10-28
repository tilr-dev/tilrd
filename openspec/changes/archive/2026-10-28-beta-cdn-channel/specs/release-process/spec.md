## MODIFIED Requirements

### Requirement: The CDN layout is one archive per platform
Per platform, `scripts/release.sh` SHALL lay out in `dist/cdn`: `releases/<version>/tilr-<version>-<platform>.tar.gz` (gzip tar holding both programs and documentation), its `.sha256` (`<hex>  <archive>`), and `releases/<platform>/latest` holding `<version>\n`. This local staging layout SHALL remain the same for both channels. On the public CDN stable SHALL use `releases/<platform>/latest` and beta SHALL use `releases/<platform>/beta`, both pointing to the same versioned archive layout.

**Verified by**: itests `UC-001` (`uc_001.py::the_installer_puts_tilrd_and_tilr_in_local_bin_without_root`), whose CDN publishes this layout (`LocalCdn.publish`).

#### Scenario: One platform built
- **WHEN** `scripts/release.sh linux-x86_64` finishes for `0.1.0-26449dd`
- **THEN** the archive holds `tilrd` and `tilr` mode 0755, `LICENSE-APACHE`, `LICENSE-MIT` and `README.md` at its top level
- **AND** `dist/cdn/releases/0.1.0-26449dd/` holds `tilr-0.1.0-26449dd-linux-x86_64.tar.gz` and its `.sha256`, and `releases/linux-x86_64/latest` is `0.1.0-26449dd`

### Requirement: Publishing uploads the archive before latest
With `--publish` and optional `--channel stable|beta` (default `stable`), `scripts/release.sh` SHALL upload each platform's archive and checksum before its selected pointer: `releases/<platform>/latest` for stable or `releases/<platform>/beta` for beta. Uploads SHALL use AWS CLI `aws s3 cp` to bucket `tilr-cdn` (`https://cdn.tilr.dev`) with the endpoint, metadata and success output specified below.

**Verified by**: `release::publishing_uploads_the_archive_and_its_checksum_before_latest`, `release::beta_publication_uploads_only_archives_checksums_and_selected_beta_pointers`, `release::beta_publication_failure_does_not_advance_the_pointer_or_publish_the_installer`.

#### Scenario: Publish two platforms
- **WHEN** `scripts/release.sh --publish linux-aarch64 linux-x86_64` runs with S3 credentials scoped to objects in `tilr-cdn`
- **THEN** each platform's archive and checksum are uploaded before its `latest`
- **AND** uploads use endpoint `https://<CLOUDFLARE_ACCOUNT_ID>.r2.cloudflarestorage.com`, region `auto`
- **AND** archives use `application/gzip`, checksums use `text/plain; charset=utf-8`, both with `public, max-age=31536000, immutable`; pointers use `text/plain; charset=utf-8` with `no-cache`
- **AND** stdout prints `published <version> for: <platforms>`

#### Scenario: Publish a candidate
- **WHEN** `scripts/release.sh --publish --channel beta linux-x86_64` runs
- **THEN** the archive and checksum are uploaded before beta; latest and install.sh are untouched

### Requirement: --upload-only uploads what earlier builds laid out
`scripts/release.sh --upload-only <platforms>` SHALL build nothing; it SHALL check that every platform given has a `dist/cdn/releases/<platform>/latest`, that all name the same version and that each archive matches its `.sha256`, then upload them as `--publish` does, with the same optional `--channel stable|beta`. Neither pointer of a platform not given SHALL change. Stable publication SHALL leave beta unchanged and beta publication SHALL leave latest unchanged.

**Verified by**: `release::upload_only_checks_what_was_built_and_leaves_other_platforms_alone`, `release::beta_publication_uploads_only_archives_checksums_and_selected_beta_pointers`.

#### Scenario: No staged platform
- **WHEN** a selected platform has no local latest file
- **THEN** upload-only fails with `nothing built for <platform>: no ...` before uploading

#### Scenario: Platforms built as different versions
- **WHEN** `linux-aarch64` was built as one version and `linux-x86_64` as another and `--upload-only` is given both
- **THEN** it says `<platform> is built as <v1>, not <v2>`, exits 1 and uploads nothing

#### Scenario: Upload a staged candidate
- **WHEN** a candidate is staged under local latest and `--upload-only --channel beta` runs for selected platforms
- **THEN** no build runs and only their archives, checksums and beta pointers are uploaded

### Requirement: The CDN gets the release last
After GitHub assets are attached, the `cdn` job SHALL verify each built platform carries `<tag version>-<commit>` and upload its completed `dist/cdn` layout with `scripts/release.sh --upload-only --channel <channel> <platforms>`, for both created releases and candidates. The channel SHALL be stable for a release and beta for a candidate. Both macOS platforms SHALL always be included; Linux arm64 SHALL remain optional.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** the `macos-aarch64`, `macos-x86_64` and `linux-x86_64` archives, checksums and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

#### Scenario: macOS publication
- **WHEN** both macOS builds complete and GitHub assets are attached
- **THEN** the CDN job uploads each macOS archive and checksum before that platform's selected channel pointer (latest for stable, beta for a candidate), without rebuilding or re-signing

#### Scenario: CDN credentials
- **WHEN** the CDN job publishes a release
- **THEN** it uses environment `cdn`, restricted to `release/*` branches, with bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets and the `CLOUDFLARE_ACCOUNT_ID` variable

#### Scenario: Candidate publication
- **WHEN** candidate GitHub assets have been attached
- **THEN** the CDN job uploads completed archives and checksums before beta for enabled platforms, without rebuilding or modifying latest or install.sh

### Requirement: A push to a release branch builds a release candidate
A push to `release/X.Y` that creates no release SHALL tag the pushed commit `vX.Y.Z-rc.N` and create a GitHub prerelease with the same archives and `.sha256` per target as a release, built and checked as a release is, the binaries naming `X.Y.Z-rc.N-<commit>`. The CDN SHALL receive the candidate archives and checksums and update only the beta pointers; latest and install.sh SHALL NOT be touched.

**Verified by**: `release::a_push_to_a_release_branch_tags_the_next_release_candidate` (the workflow step's shell with a stub GitHub CLI), `release::a_release_candidate_is_built_like_a_release_and_publishes_to_beta` (static: `.github/workflows/release.yml`).

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

### Requirement: The installer is published after the release it installs
`scripts/release.sh --publish` and `--upload-only` SHALL, for the stable channel only, upload `scripts/install.sh` to `install.sh` in bucket `tilr-cdn` (`text/x-shellscript; charset=utf-8`, `public, max-age=300`) after the `latest` of every platform given, and SHALL NOT upload it when any of those uploads failed. Beta publication SHALL NOT upload install.sh, even after successful uploads. The served installer SHALL be the one the itests run (`scripts/install.sh`).

**Verified by**: `release::the_installer_is_uploaded_after_latest` (a test of `scripts/release.sh --upload-only` with a stub `aws` on `PATH` that logs its calls).

#### Scenario: Upload order
- **WHEN** `scripts/release.sh --upload-only linux-x86_64` runs with a stub `aws` that logs each `s3 cp`
- **THEN** the log names the archive, its `.sha256`, `releases/linux-x86_64/latest` and then `install.sh`, in that order

#### Scenario: A failed upload
- **WHEN** the stub `aws` fails on `releases/linux-x86_64/latest`
- **THEN** the script exits non-zero and `install.sh` is not uploaded

#### Scenario: Candidate installer isolation
- **WHEN** beta publication succeeds
- **THEN** install.sh is not uploaded
