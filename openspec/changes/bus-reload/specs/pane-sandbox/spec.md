## ADDED Requirements

### Requirement: The sandbox survives a reload
After a reload, a sandboxed pane SHALL still be sandboxed: `PaneInfo.sandboxed` stays true and programs started in it after the reload are restricted as before.

**Verified by**: `reload::a_sandboxed_pane_stays_sandboxed_across_a_reload`.

#### Scenario: Write after reload
- **WHEN** a sandboxed pane exists, the server reloads, the client reconnects, and the pane runs `touch <o>/after`
- **THEN** `<o>/after` does not exist and `pane.list` shows `sandboxed` true
