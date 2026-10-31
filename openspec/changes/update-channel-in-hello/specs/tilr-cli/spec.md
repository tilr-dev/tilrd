## MODIFIED Requirements

### Requirement: version
`tilr version` SHALL print `tilr <version> (<platform>)`, then exactly `tilrd <HelloAck.server_version> at <target>` followed by a separate `tilrd update channel: <HelloAck.update_channel>` line when that field is non-empty. An older server's empty field SHALL omit the channel line. With nothing listening it SHALL instead print `tilrd is not running at <target>`. It SHALL exit 0 in both cases and start nothing.

**Verified by**: `client::the_first_tilr_starts_a_detached_server_and_the_next_one_uses_it`, `client::version_reports_the_configured_update_channel` and `client::version_omits_the_channel_for_an_older_server`.

#### Scenario: Versions
- **WHEN** a tilrd runs and the user runs `tilr version`
- **THEN** the first line starts with `tilr ` and the second with `tilrd `

#### Scenario: Server update channel
- **WHEN** `tilr version` connects to a server configured for `beta` (or with the channel key absent)
- **THEN** the server version line is unchanged and the next line is `tilrd update channel: beta` (or `tilrd update channel: stable` respectively)

#### Scenario: Older server
- **WHEN** `tilr version` receives a HelloAck with an empty update_channel
- **THEN** it prints the server version line without a channel line

