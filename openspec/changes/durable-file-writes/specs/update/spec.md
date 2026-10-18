## ADDED Requirements

### Requirement: An installed release is on disk before the reload
`tilrd update` SHALL sync the new executable and its directory to disk before it reports success and before the server reloads into it.

#### Scenario: Update completes
- **WHEN** `tilrd update` installs a new release
- **THEN** the executable and the rename are synced before the reload starts
