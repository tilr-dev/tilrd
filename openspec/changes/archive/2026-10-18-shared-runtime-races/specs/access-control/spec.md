## ADDED Requirements

### Requirement: A connection gone before its uid check is not refused
On the Unix socket, the host socket and the agent sockets, a connection whose peer has closed before the server could read its credentials (a probe of whether a server is there; macOS cannot read them then) SHALL be closed without the `refused a connection` warning. A connection from another uid MUST still be refused and logged.

**Verified by**: `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` (the host socket) and itests UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`, another uid still refused).

#### Scenario: A probe of the host socket
- **WHEN** a second server of the same user connects to the first one's `<server_id>.sock` and closes at once
- **THEN** the first server logs no `refused a connection` line
