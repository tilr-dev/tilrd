## MODIFIED Requirements

### Requirement: Reloading and updating the server by call
`server.reload {}` SHALL reload the server as SIGUSR1 does; its caller SHALL see either the reply or the end of its connection at the reload. `server.update {}` SHALL stream the output of `tilrd update` and end with its exit; each SHALL need the owner or the `control` action of its own name (`server.reload`, `server.update`), and be `denied` otherwise.

**Verified by**: `reload::a_reload_asked_on_the_bus_keeps_the_panes`, `update::an_update_asked_on_the_bus_streams_its_progress`, `bus_terminal::services::an_instance_without_the_rights_is_denied_the_server_methods`, `bus_terminal::services::each_server_method_needs_its_own_control_action`.

#### Scenario: Reload by call
- **WHEN** the owner calls `server.reload` while a pane runs `sleep 1000`
- **THEN** the reply arrives or the connection ends before it, the connection ends, the server reloads, and after reconnecting `pane.list` shows the pane with the same pid

#### Scenario: An instance without the right
- **WHEN** an instance whose rights let it call `server.*` but name no `control` right calls `server.reload` and `server.update`, and `server.command`
- **THEN** each is `denied` and the server does not reload

#### Scenario: One action is not the other
- **WHEN** an instance whose only `control` right is `server.update` calls `server.update`, then `server.reload`
- **THEN** the update runs to its exit, and `server.reload` is `denied`

#### Scenario: Update by call
- **WHEN** the owner calls `server.update` and the CDN's latest release is the one installed
- **THEN** the stream's stdout says it is the latest release and `End` has code 0
