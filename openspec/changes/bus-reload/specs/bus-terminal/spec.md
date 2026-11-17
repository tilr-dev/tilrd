## ADDED Requirements

### Requirement: A pane keeps its creator across a reload
A pane's `PaneInfo.created_by` SHALL be handed to the successor on a reload, so that the participant that created the pane is still its creator after the reload.

**Verified by**: `reload::a_bus_created_pane_keeps_its_creator_across_a_reload`.

#### Scenario: Across a reload
- **WHEN** `t-1` created a pane, the server reloads, and `t-1` inputs to it
- **THEN** the input is applied and `pane.list` shows `created_by` `instance:t-1`
