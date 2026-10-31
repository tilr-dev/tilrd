## ADDED Requirements

### Requirement: Check the configured release without installing it
`tilrd update --check` SHALL use the same configured channel, CDN pointer and authenticated manifest discovery as an ordinary update. It SHALL print `installed: <version>`, `available: <version>` and `channel: <stable|beta>`. It SHALL exit 0 when the versions match, 2 when they differ (including an older available version), and 1 on discovery or authentication errors, with the same diagnostics as an ordinary update. It SHALL NOT download the release archive, create or modify files, acquire the installation lock, install programs or request a reload. `tilrd update --help` SHALL describe the flag and its outcome exit codes.

**Verified by**: `update::check_reports_the_configured_release_without_changes`, `update::check_rejects_invalid_metadata_without_changes`, `update::check_help_describes_exit_codes`.

#### Scenario: Stable is the default channel
- **WHEN** the config omits `update_channel` and the stable pointer names the installed version
- **THEN** check prints both versions and channel `stable`, exits 0 and changes nothing

#### Scenario: Beta offers a different version
- **WHEN** `update_channel = "beta"` and the signed beta manifest names a different version
- **THEN** check prints both versions and channel `beta`, exits 2 and downloads no archive

#### Scenario: An older release is available
- **WHEN** the configured channel names an older version than the installed version
- **THEN** check exits 2, using the same different-version outcome

#### Scenario: Metadata cannot be trusted or fetched
- **WHEN** the CDN is unavailable or its manifest has an invalid signature
- **THEN** check fails with exit 1 and the ordinary update diagnostic, without changing files
