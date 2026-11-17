## ADDED Requirements

### Requirement: Creating a pane on the bus needs pane rights
For a caller other than the owner, `pane.create` SHALL be `denied` unless its token has `pane_profile(<the profile>)` or `pane_profile_any()` (or, without a profile, `pane_unrestricted()`), and, when the request sets `command` or `cwd`, also `pane_override()`; the denial SHALL name the missing right and no pane SHALL start.

**Verified by**: `bus_terminal::creating_a_pane_needs_pane_rights`.

#### Scenario: Another profile
- **WHEN** `t-1` has `pane = ["profile:agent"]` and calls `pane.create {profile: "demo"}`
- **THEN** the reply is `denied` naming `pane_profile`, and no pane is listed

#### Scenario: No profile
- **WHEN** `t-1` with the same rights calls `pane.create {}`
- **THEN** the reply is `denied` naming `pane_unrestricted`

#### Scenario: Override
- **WHEN** `t-1` calls `pane.create {profile: "agent", command: ["/bin/sh"]}`
- **THEN** the reply is `denied` naming `pane_override`

### Requirement: Pane access follows the creator
`PaneInfo.created_by` SHALL be the identity of the participant that created the pane (`owner`, `instance:<name>`, `token:<name>`). A caller whose token has `pane_access("own")`, or names no `pane_access`, SHALL list, watch, read, input to, resize and close only panes whose `created_by` is its identity, and SHALL receive pane events only for them; with `pane_access("any")` every pane; any other access MUST be `denied`.

**Verified by**: `bus_terminal::pane_access_follows_the_creator`.

#### Scenario: The owner's pane
- **WHEN** the owner created a pane and `t-1` (no `pane_access`) calls `pane.read` on it and subscribes to `pane.*`
- **THEN** the read is `denied`, `pane.list` from `t-1` does not include it, and `t-1` receives no event of it

### Requirement: A pane outlives the participant that made it
A pane SHALL keep running when the participant that created it disconnects, stops, crashes or is reloaded; that participant's next connection SHALL access it under the same rights.

**Verified by**: `bus_terminal::a_pane_outlives_the_participant_that_made_it`.

#### Scenario: Instance reloaded
- **WHEN** `t-1` created a pane running `sleep 1000` and `tilr bus reload t-1` runs
- **THEN** the pane still runs and the new `t-1` process can call `pane.read` on it
