## MODIFIED Requirements

### Requirement: The updater selects a release channel
`tilrd update` SHALL read `update_channel = "stable"|"beta"` from server.toml, default to `stable` if the file or key is absent, and reject other values before CDN access or installation changes. `tilr update` SHALL follow the server's update channel. Argument parsing SHALL reject `--channel`. No environment variable SHALL select the channel. Missing beta SHALL fail without stable fallback. The installer and `fetch-release` SHALL remain on stable.

**Verified by**: `update::update_selects_beta_without_changing_the_stable_default`, `update::update_rejects_invalid_channels_and_does_not_fall_back_from_beta`, `update::configured_beta_stays_current_when_stable_is_older`, `update::update_rejects_the_removed_channel_flag_before_contacting_cdn`.

#### Scenario: A beta candidate is installed
- **WHEN** latest names a stable release, beta names a candidate and `tilrd update` runs with `update_channel = "beta"` in server.toml
- **THEN** both installed programs come from the candidate archive

#### Scenario: Stable remains the default
- **WHEN** server.toml is absent, has no update_channel key, or sets update_channel to "stable"
- **THEN** the update reads latest, even if beta names a different candidate

#### Scenario: Missing beta
- **WHEN** beta is missing while latest exists
- **THEN** `tilrd update` configured for beta fails and leaves both programs unchanged

#### Scenario: Invalid channel
- **WHEN** server.toml sets `update_channel = "rc"` and `tilrd update` runs
- **THEN** configuration loading fails with a message identifying the invalid channel before any CDN request and changes nothing

#### Scenario: The channel flag is removed
- **WHEN** `tilrd update --channel beta` runs
- **THEN** argument parsing fails before any CDN request or installation change
