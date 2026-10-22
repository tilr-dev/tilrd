## MODIFIED Requirements

### Requirement: A pane that cannot start is reported to its creator
When a pane cannot be started (its process cannot be spawned, its program cannot be run, or its process dies during startup) the asking client SHALL get an `Error` whose message starts with `create pane: `, and no pane SHALL be listed for the request.

**Verified by**: `panes::a_pane_whose_command_cannot_start_is_reported_to_its_creator`.

#### Scenario: A program that does not exist
- **WHEN** a client sends `CreatePane` with `command` `["/no/such/program"]`
- **THEN** it gets an `Error` starting with `create pane: ` and no `PaneCreated`

### Requirement: When a pane was used last
`PaneInfo.used_at_ms` SHALL be the Unix time in milliseconds when a client last subscribed to the pane or sent it key, mouse, paste, raw or scroll input, and `created_at_ms` until then. A change of `used_at_ms` alone SHALL NOT push a `PaneList`.

**Verified by**: `panes::used_at_ms_follows_subscribe_and_input`.

#### Scenario: Input marks the pane used
- **WHEN** a client sends `KeyInput` to a pane and then sends `ListPanes`
- **THEN** that pane's `used_at_ms` is not earlier than the time the key was sent
