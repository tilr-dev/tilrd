## MODIFIED Requirements

### Requirement: Panes know their server and its host socket
Every pane SHALL have `TILR_PANE` (its id), `TILR_SERVER` (`HelloAck.server_id`) and `TILR_SOCK` (the host socket) in its environment and the server's `tilr` first on `PATH`. The host socket SHALL be in the runtime directory (macOS: `~/Library/Application Support/tilr` unless `TILR_RUNTIME_DIR` or `XDG_RUNTIME_DIR` is set; a path over 103 bytes goes under `/tmp/tilr-<uid>/`) and named `<server_id>.sock`, except that a server SHALL use `<server_id>-<pid>.sock` when a socket of that name exists (its own, from before a reload) or when another live server accepts connections at `<server_id>.sock`. Two servers of one user running at once MUST each have a host socket of their own.

**Verified by**: `files::tilr_edit_in_a_pane_asks_the_clients_watching_it_to_open_the_file` and `files::two_servers_of_one_user_keep_their_host_sockets`.

#### Scenario: Pane environment
- **WHEN** a client opens a pane and runs `test "$TILR_PANE" = <pane id> && test "$TILR_SERVER" = <server_id> && test -S "$TILR_SOCK"`
- **THEN** the command exits 0

#### Scenario: Two servers of one user
- **WHEN** two `tilrd` of one user run on different Unix sockets with one state directory, and each has a pane with a client watching it
- **THEN** the panes' `TILR_SOCK` differ, the first server's is `<server_id>.sock`, and `tilr edit` works in a pane of each

#### Scenario: Reload keeps the path
- **WHEN** the second server (on `<server_id>-<pid>.sock`) reloads and a pane created before the reload runs `tilr edit`
- **THEN** it reaches that server's clients

### Requirement: The host socket is private and goes with the server
The host socket SHALL be mode 0600 in a 0700 directory, replacing a stale socket at its path (one no server accepts on) but refusing a file that is not a socket; the server MUST remove it when it stops (a reload keeps it), and only if the file at that path is still the socket it bound, so a server never removes another server's socket. At start the server SHALL remove `<server_id>-<pid>.sock` files of pids that are not running. If it cannot be bound the server SHALL run on without host events (no `"host-events"` feature).

**Verified by**: `files::tilr_edit_fails_with_no_client_watching_or_outside_a_pane`, `files::two_servers_of_one_user_keep_their_host_sockets` and UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`).

#### Scenario: Stop removes the socket
- **WHEN** the server is sent SIGTERM and exits
- **THEN** the path in `$TILR_SOCK` no longer exists

#### Scenario: Stopping one of two servers
- **WHEN** of two servers of one user the one on `<server_id>-<pid>.sock` stops, and then the other
- **THEN** after the first stop the other's `<server_id>.sock` still exists and `tilr edit` works in its panes

#### Scenario: Modes
- **WHEN** a pane's `$TILR_SOCK` is looked at
- **THEN** the socket is 0600 and its directory 0700
