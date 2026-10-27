## MODIFIED Requirements

### Requirement: Every pane has its own agent socket
While forwarding is on, every pane SHALL get a Unix socket of its own at `agent/<pid>/<pane id>.sock` in the runtime directory (`$XDG_RUNTIME_DIR/tilr`, on macOS `~/Library/Application Support/tilr`, `/tmp/tilr-<uid>` where that path is too long for a socket), `<pid>` being the server's, and `SSH_AUTH_SOCK` in the pane's environment MUST point at it. The socket MUST have mode 0600 and the `agent/<pid>/` and `agent/` directories (and the runtime directory above them) mode 0700.

**Verified by**: `agent::a_pane_has_ssh_auth_sock_pointing_at_its_own_socket`.

#### Scenario: Two panes
- **WHEN** a client opens two panes
- **THEN** each pane's `SSH_AUTH_SOCK` ends in `agent/<the server's pid>/<its pane id>.sock`, a socket of mode 0600 in a directory of mode 0700, and the two paths differ

### Requirement: Sockets a killed server left are swept at start
A server that starts fresh (not resuming after a reload) SHALL remove the `agent/<pid>/` directories in the runtime directory whose pid is not running, logging `removed N agent socket(s) a server that is gone left`, and MUST leave the directory of every running server of the same user alone: it MUST NOT connect to their sockets (no stream is opened to their clients, nothing is logged by them) nor remove one, even while it is being made and does not accept yet.

**Verified by**: `agent::the_sockets_a_killed_server_left_are_removed_when_the_next_starts`, `agent::a_server_starting_leaves_the_agent_sockets_of_a_live_one_alone` and itests `UC-036` (`uc_036.py::agent_sockets_left_by_a_killed_server_are_removed_at_the_next_start`).

#### Scenario: SIGKILL, then a new server
- **WHEN** a server with a pane is killed with SIGKILL while another server of the user runs with a pane of its own, and a new server starts
- **THEN** the killed server's `agent/<pid>/<pane>.sock` and its `agent/<pid>/` directory are gone and the live server's socket still exists

#### Scenario: A second server starts next to a live one
- **WHEN** a server has a pane watched by a client with `"agent"`, and a second server of the user starts in the same runtime directory
- **THEN** the first server's client receives no `StreamOpen`, the first server does not log `agent socket: refused a connection`, and the pane's socket is the same file and still serves the agent

### Requirement: A reload ends open streams and keeps the socket path
A reload or update SHALL end every open agent stream (the program reads end of file) and make each pane's socket again at the same path, so the shell's `SSH_AUTH_SOCK` stays valid and the next connection reaches a client that reconnects and watches the pane. The path is the one the pane got: `agent/<pid>/<pane id>.sock` (the pid survives the exec), or `agent/<pane id>.sock` for a pane a server from before the per-server directories started, while that socket file is there.

**Verified by**: `agent::the_panes_socket_works_after_a_reload` and itests `UC-064` (`uc_064.py::an_agent_stream_open_during_an_update_ends_for_the_program_and_the_next_one_works`).

#### Scenario: SIGUSR1 with a stream open
- **WHEN** a program has a stream open and the server reloads
- **THEN** the program reads end of file
- **AND** after the client reconnects and watches the pane, `SSH_AUTH_SOCK` is the same path and a new connection gets `StreamOpen` and carries bytes both ways
