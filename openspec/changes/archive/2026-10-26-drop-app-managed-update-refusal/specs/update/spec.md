## REMOVED Requirements

### Requirement: A server managed by Tilr.app is not updated here
**Reason**: The Mac app uses the ordinary server updater; a legacy marker must no longer prevent release installation.
**Migration**: Run `tilrd update` normally. Existing marker files may remain in place.

## ADDED Requirements

### Requirement: Legacy app markers do not affect updates
`tilrd update` SHALL ignore `app-managed.json` next to the executable and use the ordinary release discovery, verification, installation, and reload path.

**Verified by**: `update::update_installs_the_latest_release_with_a_legacy_app_marker`.

#### Scenario: Update with a legacy app marker
- **WHEN** `app-managed.json` is next to the executable and the CDN offers a newer valid release
- **THEN** the update exits 0, installs both release executables, and leaves the marker unchanged
