## ADDED Requirements

### Requirement: Check the configured release without installing it
`tilrd update --check` SHALL reuse ordinary update's configured channel and authenticated release discovery, print installed and available versions and the channel, and exit 0 for equal versions, 2 for different versions or 1 on errors. It SHALL fetch only metadata and SHALL NOT change files, lock the installation or request reload. Its help SHALL describe the flag and outcome exit codes.

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

#### Scenario: Output and help
- **WHEN** check successfully authenticates the configured release
- **THEN** stdout contains `installed: <version>`, `available: <version>` and `channel: <stable|beta>` on separate lines
- **AND** `tilrd update --help` describes `--check` and exit codes 0 (same version), 2 (different version) and 1 (error)
