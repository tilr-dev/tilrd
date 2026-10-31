## MODIFIED Requirements

### Requirement: The updater selects a release channel
`tilrd update` SHALL read `update_channel = "stable"|"beta"` from its server.toml configuration, default to `stable` when the file or key is absent, and reject any other value with a configuration error before contacting the CDN or changing the installation. The `--channel` flag SHALL be rejected by argument parsing. No environment variable SHALL select the channel. A missing beta pointer SHALL fail without falling back to stable. The installer, `tilr update` and `fetch-release` SHALL retain their stable behavior.

**Verified by**: `update::update_selects_beta_without_changing_the_stable_default`, `update::update_rejects_invalid_channels_and_does_not_fall_back_from_beta`.

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
