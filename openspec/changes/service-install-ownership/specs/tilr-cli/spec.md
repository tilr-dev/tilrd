## ADDED Requirements

### Requirement: Install does not replace a unit it did not write
`tilr service install` SHALL NOT overwrite a systemd unit, LaunchAgent or crontab line that lacks its marker; it MUST exit non-zero naming the file and leave it unchanged.

#### Scenario: Foreign tilr.service
- **WHEN** `~/.config/systemd/user/tilr.service` exists without the marker
- **THEN** install exits non-zero and the file is unchanged
