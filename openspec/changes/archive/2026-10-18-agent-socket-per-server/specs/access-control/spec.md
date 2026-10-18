## MODIFIED Requirements

### Requirement: The host-events and agent sockets check the peer's uid
The host-events socket (`$TILR_SOCK` in each pane, see `clipboard-host-events`) and each pane's SSH agent socket (`agent/<pid>/<pane id>.sock`, see `ssh-agent-forwarding`) SHALL be created 0600 in a directory of mode 0700 that belongs to the user, and the server MUST refuse every connection on them whose peer uid is not its own, logging "host socket: refused a connection" or "agent socket: refused a connection", even where the permissions were opened.

**Verified by**: `agent::a_pane_has_ssh_auth_sock_pointing_at_its_own_socket` (modes) and itests UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`).

#### Scenario: Another user sends tilr edit
- **WHEN** another uid connects to a pane's `$TILR_SOCK`, with the permissions opened, and sends a request
- **THEN** the connection is refused and no client receives a `HostEvent`

#### Scenario: Agent socket modes
- **WHEN** a pane is created with agent forwarding on
- **THEN** its `SSH_AUTH_SOCK` is mode 0600 in a directory of mode 0700
