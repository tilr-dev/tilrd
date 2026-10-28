## MODIFIED Requirements

### Requirement: Update metadata must be authenticated
The updater SHALL verify Ed25519 over the envelope's exact UTF-8 `payload` using its embedded key. Metadata SHALL be at most 16 KiB with a hex signature. The payload SHALL bind a valid version, requested channel (`stable` or `beta`) and platform, expected archive basename and 64-character hex SHA-256. Missing, malformed, mismatched or untrusted metadata SHALL fail before any archive request or installation change. Unsigned checksum fallback and runtime trust-key overrides SHALL be rejected.

**Verified by**: `update::update_requires_authentic_metadata_before_downloading`, `update::update_rejects_cross_channel_replay_before_downloading`.

#### Scenario: A forged manifest
- **WHEN** a CDN serves a manifest signed by another key or changes its signed payload
- **THEN** the update exits non-zero, makes no archive request and leaves both installed programs unchanged

#### Scenario: An authentic release
- **WHEN** the CDN serves correctly signed metadata and an archive matching its authenticated checksum
- **THEN** the update installs the two verified programs

#### Scenario: Missing signed metadata
- **WHEN** legacy pointers and checksum files exist but the signed manifest is absent
- **THEN** the update refuses without an unsigned fallback

#### Scenario: A manifest replayed across channels
- **WHEN** the CDN serves an authentic beta manifest through the stable pointer or an authentic stable manifest through the beta pointer
- **THEN** the update exits non-zero before requesting an archive and leaves both installed programs unchanged

#### Scenario: An older release in the requested channel
- **WHEN** the selected pointer names an older release with authentic metadata for the requested channel
- **THEN** the updater accepts the metadata and installs the older release; replay within a channel remains possible

