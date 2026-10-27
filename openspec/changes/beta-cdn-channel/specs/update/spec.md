## MODIFIED Requirements

### Requirement: The latest release is read from the CDN
`tilrd update` SHALL read `<cdn>/releases/<platform>/latest` for the stable channel or `<cdn>/releases/<platform>/beta` for the beta channel (the platform as `--version` names it) and SHALL refuse a trimmed value that is empty or has characters other than ASCII letters, digits, `.`, `-`, `+`, `_` (`... names a release that is not a version: ...`). It SHALL then fetch `releases/<version>/tilr-<version>-<platform>.tar.gz.sha256` and download `releases/<version>/tilr-<version>-<platform>.tar.gz`, printing `downloading tilr <version> (<platform>)`.

**Verified by**: `update::update_installs_nothing_when_the_cdn_misbehaves`, `update::update_installs_the_latest_release_and_the_running_server_keeps_its_shells`.

#### Scenario: A login page instead of the CDN
- **WHEN** `releases/<platform>/latest` answers `<html><body>Sign in</body></html>`
- **THEN** the update exits non-zero with `not a version` on stderr and the executable is unchanged

## ADDED Requirements

### Requirement: The updater selects a release channel
`tilrd update` SHALL accept `--channel stable|beta`, default to `stable`, and reject any other channel before downloading or changing the installation. No environment variable SHALL select the channel. A missing beta pointer SHALL fail without falling back to stable. The installer, `tilr update` and `fetch-release` SHALL retain their stable behavior.

**Verified by**: `update::update_selects_beta_without_changing_the_stable_default`, `update::update_rejects_invalid_channels_and_does_not_fall_back_from_beta`.

#### Scenario: A beta candidate is installed
- **WHEN** latest names a stable release, beta names a candidate and `tilrd update --channel beta` runs
- **THEN** both installed programs come from the candidate archive

#### Scenario: Stable remains the default
- **WHEN** the channel is omitted or explicitly stable
- **THEN** the update reads latest, even if beta names a different candidate

#### Scenario: Missing beta
- **WHEN** beta is missing while latest exists
- **THEN** `tilrd update --channel beta` fails and leaves both programs unchanged

#### Scenario: Invalid channel
- **WHEN** `tilrd update --channel rc` runs
- **THEN** argument parsing fails and changes nothing
