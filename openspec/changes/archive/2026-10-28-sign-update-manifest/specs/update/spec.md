## MODIFIED Requirements

### Requirement: The latest release is read from the CDN
`tilrd update` SHALL read `<cdn>/releases/<platform>/latest` (stable) or `beta`, using the platform from `--version`. The trimmed version SHALL be nonempty and contain only ASCII letters, digits, `.`, `-`, `+`, `_`. Before accepting even an up-to-date version, it SHALL authenticate the adjacent `<latest|beta>.manifest.json`, require its version to match the pointer, and download only its named archive.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves`, `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: A login page instead of the CDN
- **WHEN** `releases/<platform>/latest` answers `<html><body>Sign in</body></html>`
- **THEN** the update exits non-zero with `not a version` on stderr and the executable is unchanged

#### Scenario: Download a valid release
- **WHEN** the selected pointer names a valid version
- **THEN** stdout prints `downloading tilr <version> (<platform>)` before fetching the archive

### Requirement: The download must match its checksum
The archive SHALL be at most 256 MiB and SHALL match the SHA-256 authenticated by its signed manifest (64 hexadecimal characters, case-insensitive); otherwise the update SHALL fail with `the download does not match its checksum, nothing was installed`, leaving the executable as it was and nothing next to it.

**Verified by**: `update::update_installs_nothing_that_fails_its_checksum_or_does_not_run`.

#### Scenario: Tampered archive
- **WHEN** the archive on the CDN is not the file its checksum was made of
- **THEN** stderr says `does not match its checksum`, the exit status is non-zero, and the directory holds only the old executable

### Requirement: A CDN that misbehaves installs nothing
A download cut short, release files missing after `latest` names them (no manifest: `<cdn> has no release <version> for <platform>`), an invalid manifest signature, or a CDN that does not answer SHALL make the update exit non-zero (`download ...` for a cut download) and leave the executable unchanged with nothing next to it.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves`.

#### Scenario: Download cut short
- **WHEN** the connection breaks off halfway through the archive
- **THEN** stderr mentions `download`, the exit status is non-zero, and the directory holds only the old executable

### Requirement: Only the release archive is looked for
`tilrd update` SHALL look for a release only as `tilr-<version>-<platform>.tar.gz`; it SHALL NOT fall back to the loose files of releases from before the archive (`tilrd-<platform>`, `tilr-server-<platform>`), and a version without signed metadata SHALL fail with `<cdn> has no release <version> for <platform>`. **BREAKING** (2a8b425): a server installed from before the archive, which asks for the loose files, finds no new release and is reinstalled once with the installer.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves` (the `has no release 9.9.9-test` step).

#### Scenario: Old layout only
- **WHEN** the CDN's `latest` names a version that has no signed manifest
- **THEN** the update exits non-zero with `has no release <version>` and the directory holds only the old executable

## ADDED Requirements

### Requirement: Update metadata must be authenticated
The updater SHALL verify Ed25519 over the manifest envelope's exact UTF-8 `payload` string using its embedded key. Metadata SHALL be at most 16 KiB, with a hex signature. The payload SHALL bind a valid version, requested platform, expected archive basename and 64-character hex SHA-256. Missing, malformed, mismatched or untrusted metadata SHALL fail before any archive request or installation change. No unsigned checksum fallback or runtime trust-key override SHALL be accepted.

**Verified by**: `update::update_requires_authentic_metadata_before_downloading`.

#### Scenario: A forged manifest
- **WHEN** a CDN serves a manifest signed by another key or changes its signed payload
- **THEN** the update exits non-zero, makes no archive request and leaves both installed programs unchanged

#### Scenario: An authentic release
- **WHEN** the CDN serves correctly signed metadata and an archive matching its authenticated checksum
- **THEN** the update installs the two verified programs

#### Scenario: Missing signed metadata
- **WHEN** legacy pointers and checksum files exist but the signed manifest is absent
- **THEN** the update refuses without an unsigned fallback
