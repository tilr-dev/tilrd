## MODIFIED Requirements

### Requirement: Publishing uploads the archive before latest
With `--publish`, `scripts/release.sh` SHALL upload with AWS CLI `aws s3 cp` through `https://<CLOUDFLARE_ACCOUNT_ID>.r2.cloudflarestorage.com` with region `auto` to bucket `tilr-cdn` (`https://cdn.tilr.dev`), per platform: the archive (`application/gzip`) and its `.sha256` (`text/plain; charset=utf-8`) with `public, max-age=31536000, immutable`, and only then `releases/<platform>/latest` with `no-cache`, printing `published <version> for: <platforms>`.

**Verified by**: `release::publishing_uploads_the_archive_and_its_checksum_before_latest`.

#### Scenario: Publish two platforms
- **WHEN** `scripts/release.sh --publish linux-aarch64 linux-x86_64` runs with S3 credentials scoped to objects in `tilr-cdn`
- **THEN** each platform's archive and checksum are uploaded before its `latest`

### Requirement: The CDN gets the release last
After the GitHub release has its assets, the `cdn` job (environment `cdn`, restricted to `main`, holding bucket-scoped `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` secrets and the `CLOUDFLARE_ACCOUNT_ID` variable) SHALL check every Linux platform built carries `<tag version>-<commit>` and run `scripts/release.sh --upload-only <platforms>` on the `dist/cdn` the Linux builds laid out; with arm64 off only `linux-x86_64` is uploaded and `releases/linux-aarch64/latest` is left as it is.

**Verified by**: `release::the_cdn_job_runs_after_the_assets_and_uploads_only_what_was_built` (static: `.github/workflows/release.yml`).

#### Scenario: arm64 off
- **WHEN** `RELEASE_LINUX_ARM64` is not `true` and a release is made
- **THEN** only the `linux-x86_64` archive, checksum and `latest` are uploaded and `releases/linux-aarch64/latest` is unchanged

### Requirement: The installer is published after the release it installs
`scripts/release.sh --publish` and `--upload-only` SHALL upload `scripts/install.sh` to `install.sh` in bucket `tilr-cdn` (`text/x-shellscript; charset=utf-8`, `public, max-age=300`) after the `latest` of every platform given, and SHALL NOT upload it when any of those uploads failed. The served installer SHALL be the one the itests run (`scripts/install.sh`).

**Verified by**: `release::the_installer_is_uploaded_after_latest` (a test of `scripts/release.sh --upload-only` with a stub `aws` on `PATH` that logs its calls).

#### Scenario: Upload order
- **WHEN** `scripts/release.sh --upload-only linux-x86_64` runs with a stub `aws` that logs each `s3 cp`
- **THEN** the log names the archive, its `.sha256`, `releases/linux-x86_64/latest` and then `install.sh`, in that order

#### Scenario: A failed upload
- **WHEN** the stub `aws` fails on `releases/linux-x86_64/latest`
- **THEN** the script exits non-zero and `install.sh` is not uploaded
