## MODIFIED Requirements

### Requirement: The Unix socket checks the peer's uid
For every connection accepted on the Unix socket the server SHALL read the peer's credentials (`SO_PEERCRED` on Linux, `getpeereid` on macOS). A per-user server MUST close at once, before any handshake, a connection whose uid is not the server's own, logging "refused a connection on the Unix socket", even where the directory and socket permissions were opened by hand. A server started with `--system` SHALL instead make the peer's uid the connection's user and MUST refuse, with the same log line, only a peer whose credentials cannot be read or whose uid has no password entry.

**Verified by**: `local::another_user_is_turned_away_even_where_the_permissions_let_it_in` and itests UC-034 (`uc_034.py::another_user_cannot_use_the_socket`); the system-server scenarios are not tested yet (planned itests UC-03C).

#### Scenario: Permissions keep another user out
- **WHEN** another uid runs `tilrd ps` against the socket of a per-user server with the default modes
- **THEN** it fails

#### Scenario: Permissions opened by hand
- **WHEN** the directories are made 0755 and the socket 0666, and another uid connects
- **THEN** the connection is refused, the server logs "refused a connection on the Unix socket", and the user's own `tilrd ps` still succeeds

#### Scenario: Two users on a system server
- **WHEN** alice and bob, both in the group `tilr`, connect to `/run/tilr/tilrd.sock` of a server started with `--system`
- **THEN** each receives `HelloAck` with `user` naming themselves

#### Scenario: Not in the group
- **WHEN** carol, not in the group `tilr`, runs `tilrd ps --socket /run/tilr/tilrd.sock`
- **THEN** it fails with `Permission denied` (what `tilr` then does: daemon-lifecycle "tilr outside the group tilr is told so")

### Requirement: The host-events and agent sockets check the peer's uid
The host-events socket (`$TILR_SOCK` in each pane, see `clipboard-host-events`) and each pane's SSH agent socket (`agent/<pid>/<pane id>.sock`, see `ssh-agent-forwarding`) SHALL be created 0600 in a directory of mode 0700 that belongs to the user, and the server MUST refuse every connection on them whose peer uid is not its own, logging "host socket: refused a connection" or "agent socket: refused a connection", even where the permissions were opened. On a server started with `--system` these sockets SHALL instead be 0600 and owned by that user in the root-owned `/run/tilr/users/<uid>/` (see daemon-lifecycle "Per-user directories of a system server"), and the uid they accept SHALL be that user's (and root's); a host event or agent stream SHALL go only to connections of that user.

**Verified by**: `agent::a_pane_has_ssh_auth_sock_pointing_at_its_own_socket` (modes) and itests UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`); the system-server scenarios are not tested yet (planned itests UC-035 extended).

#### Scenario: Another user sends tilr edit
- **WHEN** another uid connects to a pane's `$TILR_SOCK`, with the permissions opened, and sends a request
- **THEN** the connection is refused and no client receives a `HostEvent`

#### Scenario: Agent socket modes
- **WHEN** a pane is created with agent forwarding on, on a per-user server
- **THEN** its `SSH_AUTH_SOCK` is mode 0600 in a directory of mode 0700

#### Scenario: Bob writes to alice's host socket on a system server
- **WHEN** bob connects to the `$TILR_SOCK` of alice's pane on a system server and sends `tilr edit`
- **THEN** the connection is refused, and neither alice's nor bob's clients receive a `HostEvent`

#### Scenario: Alice's clipboard stays hers
- **WHEN** alice's pane writes the clipboard (OSC 52) and alice and bob are both connected
- **THEN** only alice's connections receive the clipboard write

## ADDED Requirements

### Requirement: A system server keeps a token per user
A server started with `--system` SHALL have no server-wide token; it SHALL keep one token per user in `/var/lib/tilr/users/<uid>/token` (64 hex characters, 0600, owned by root), made the first time that user asks for it. `tilr token` on the system socket SHALL print the token of the connection's user; `--new` SHALL replace it and close that user's TCP connections. Root MUST have no token.

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Bob's token is not alice's
- **WHEN** bob runs `tilr token`
- **THEN** he gets a token other than alice's, and `/var/lib/tilr/users/<bob's uid>/token` is 0600 owned by root

#### Scenario: Root asks for a token
- **WHEN** root runs `tilr token` on the system socket
- **THEN** it fails with `root has no token: a pane as root is for the Unix socket only`

### Requirement: A TCP Hello names its user by the token
On a server started with `--system`, a TCP `Hello` SHALL be accepted when its token is one user's token, and the connection's user is then that user. The errors for no or a wrong token are those of "TCP clients present the token in Hello".

**Verified by**: not tested yet (a test is named in tasks.md).

#### Scenario: Alice's token
- **WHEN** alice runs `tilr token` on the system socket and a TCP client sends `Hello` with it
- **THEN** it receives `HelloAck` with `user` `alice` and lists only alice's panes

#### Scenario: A token of nobody
- **WHEN** a TCP client sends `Hello` with a random 64-character token
- **THEN** it receives `Error` "wrong access token" and the connection closes
