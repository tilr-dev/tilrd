## ADDED Requirements

### Requirement: The installer is published after the release it installs
`scripts/release.sh --publish` and `--upload-only` SHALL upload `scripts/install.sh` to `install.sh` in bucket `tilr-cdn` (`text/x-shellscript; charset=utf-8`, `public, max-age=300`) after the `latest` of every platform given, and SHALL NOT upload it when any of those uploads failed. The served installer SHALL be the one the itests run (`scripts/install.sh`).

**Verified by**: `release::the_installer_is_uploaded_after_latest` (a test of `scripts/release.sh --upload-only` with a stub `wrangler` on `PATH` that logs its calls).

#### Scenario: Upload order
- **WHEN** `scripts/release.sh --upload-only linux-x86_64` runs with a stub `wrangler` that logs each `r2 object put`
- **THEN** the log names the archive, its `.sha256`, `releases/linux-x86_64/latest` and then `install.sh`, in that order

#### Scenario: A failed upload
- **WHEN** the stub `wrangler` fails on `releases/linux-x86_64/latest`
- **THEN** the script exits non-zero and `install.sh` is not uploaded
