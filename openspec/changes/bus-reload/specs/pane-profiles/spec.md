## ADDED Requirements

### Requirement: A pane keeps its profile across a reload
A pane's `PaneInfo.profile` and the command, directory, variables and sandbox it was started with SHALL be handed to the successor on a reload and SHALL NOT be read again from the profile file.

**Verified by**: `reload::a_pane_keeps_its_profile_across_a_reload`.

#### Scenario: Profile removed before a reload
- **WHEN** a pane was created with profile `demo`, `demo.toml` is deleted, and the server reloads
- **THEN** after reconnecting `pane.list` shows the pane with `profile` `demo`, and its shell still has the profile's `[env]` variables
