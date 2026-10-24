# access-control Specification

## Purpose

Defines who may connect to `tilrd` and how they prove it: the access token on the TCP port, the browser Origin allowlist, the Unix socket that needs no token but checks the peer's uid, the other per-user sockets (host events, SSH agent), the handshake timeout, and what Cloud's tunnel changes about it. Where the sockets are and how the server takes them lives in `daemon-lifecycle`; the token checks of `/socks` and `/metrics` in `socks-proxy` and `metrics`; the tunnel itself in `cloud-tunnels`.

## Requirements

### Requirement: The access token is made once and kept for the user only
A server that asks for a token SHALL, the first time it runs, make one from 32 random bytes written as 64 lowercase hex characters and keep it in the file `token` in the state directory (`$TILR_STATE_DIR`, else `$XDG_STATE_HOME/tilr`, else `~/.local/state/tilr`). The file MUST be created 0600 and the state directory 0700 if the server makes it. A server MUST reuse the stored token on every later start and MUST NOT replace a token file that exists.

**Verified by**: `auth::the_token_is_kept_for_the_user_only_and_across_restarts` and `auth::a_client_needs_the_access_token`.

#### Scenario: First start
- **WHEN** a server starts with an empty state directory
- **THEN** `<state>/token` holds a 64-character token, mode 0600, in a directory of mode 0700

#### Scenario: Restart
- **WHEN** the server is killed and started again with the same state directory
- **THEN** it asks for the same token as before

### Requirement: Showing the token
`tilrd token` SHALL print the token (making it if there is none) followed by a newline and exit 0. For users, `tilr token` SHALL run that command inside the running server (`ExecStart` with `server_command`, see the `tilr-cli` and `exec` specs) and print its output. A server command MUST be accepted only on the Unix socket, even with `--no-exec`; over TCP, with the token or not, it MUST be refused with an `ExecStarted.error` saying it is for the Unix socket only.

**Verified by**: `auth::the_token_is_kept_for_the_user_only_and_across_restarts`, `local::server_commands_run_for_local_clients_only` and `client::server_commands_run_in_the_server`.

#### Scenario: tilrd token
- **WHEN** `tilrd token` runs with the server's state directory
- **THEN** its stdout, trimmed, is the token the server asks for

#### Scenario: Server command over TCP
- **WHEN** a TCP client that presented the token sends `ExecStart { argv: ["token"], server_command: true }`
- **THEN** `ExecStarted.error` contains "Unix socket only" and no token is printed

### Requirement: TCP clients present the token in Hello
On the TCP port, a server that asks for a token SHALL accept a `Hello` only when `Hello.token` equals its token, compared in time that does not depend on where they differ. A `Hello` without a token MUST be answered `Error { message: "this server asks for its access token" }`, one with another token `Error { message: "wrong access token" }`, and the connection then closed, with nothing the client sent acted on. This holds whatever address the server listens on, loopback or not.

**Verified by**: `auth::a_client_needs_the_access_token` and itests UC-031 (`uc_031.py::listening_on_every_ipv4_address_asks_for_the_token`, `uc_031.py::listening_on_every_address_takes_ipv4_too_and_asks_for_the_token`).

#### Scenario: No token
- **WHEN** a TCP client sends `Hello` with an empty `token`
- **THEN** it receives `Error` "this server asks for its access token" and the connection closes

#### Scenario: Wrong token
- **WHEN** a TCP client sends `Hello` with a token that differs in one character
- **THEN** it receives `Error` "wrong access token" and the connection closes

#### Scenario: Right token
- **WHEN** a TCP client sends `Hello` with the token
- **THEN** it receives `HelloAck`

### Requirement: Running without authentication
`tilrd --no-auth` SHALL ask no client for a token (any `Hello.token` is accepted), MUST make no token, and MUST log the warning "no authentication: anyone who can reach this server gets a shell". Such a server MUST take no tunnel from tilr Cloud (see `cloud-tunnels`). The Origin check MUST still apply.

**Verified by**: `auth::a_server_started_without_authentication_asks_for_no_token` and `cloud::a_server_without_authentication_takes_no_tunnel`.

#### Scenario: Hello without a token
- **WHEN** a client sends `Hello` with an empty token to a server started with `--no-auth`
- **THEN** it receives `HelloAck`
- **AND** the server's log says "no authentication"

### Requirement: HTTP requests on the port carry the token in a header or the query
A request on the TCP port that has no `Hello` (the SOCKS WebSocket at `/socks`, `GET /metrics`) SHALL present the token as `Authorization: Bearer <token>`, or, for a page that cannot set headers, as `?token=<token>` in the URL. Without it, or with another, the server MUST answer HTTP 401 with `WWW-Authenticate: Bearer` and serve nothing. The rules specific to each (and `/metrics` on a server without a token) are in the `socks-proxy` and `metrics` specs.

**Verified by**: `socks::the_socks_websocket_needs_the_access_token`, `socks::a_page_that_cannot_set_headers_gives_the_token_in_the_query` and `metrics::metrics_ask_for_the_access_token`.

#### Scenario: Token in the query
- **WHEN** a page opens `/socks?token=<token>` without an `Authorization` header
- **THEN** the WebSocket is accepted

#### Scenario: No token
- **WHEN** `GET /metrics` carries neither header nor query token on a server with a token
- **THEN** the answer is 401 "access token required"

### Requirement: Browser origins are checked on every handshake
A WebSocket handshake or HTTP request that carries an `Origin` header SHALL be accepted only when the origin equals one entry of the allowed list exactly; otherwise the server MUST answer HTTP 403 ("origin not allowed") and log "rejected connection from disallowed origin". A request without `Origin` (a program that is not a browser) MUST pass this check. The check MUST apply on the TCP port and the Unix socket, to the main protocol, `/socks` and `/metrics`, token or not.

**Verified by**: `panes::a_browser_page_connects_only_from_an_allowed_origin` and `socks::the_socks_websocket_comes_only_from_an_allowed_origin`.

#### Scenario: Page from another site
- **WHEN** a server started with `--allow-origin http://app.test` gets a handshake with `Origin: http://evil.test`
- **THEN** the handshake fails with 403

#### Scenario: Allowed page and plain program
- **WHEN** one handshake carries `Origin: http://app.test` and another carries no `Origin`
- **THEN** both connect and are answered `HelloAck`

### Requirement: Where the allowed origins come from
The allowed origins SHALL be the `--allow-origin` values (repeatable) if any; else `allow_origins` in the configuration file (`--config` / `TILR_CONFIG`, else `server.toml` in `$TILR_CONFIG_DIR`, `$XDG_CONFIG_HOME/tilr` or `~/.config/tilr`); else `tilr://app`, `tilr://tilr`, `http://127.0.0.1:7891`, `http://localhost:7891`. Each source MUST replace the next, not add to it. A configuration file that does not parse or names an unknown key MUST stop the server with an error naming the file.

**Verified by**: `panes::the_configuration_file_names_the_allowed_origins` and unit tests `the_file_sets_the_origins_and_the_command_line_wins` and `an_unknown_key_is_an_error` in crates/tilrd/src/config.rs.

#### Scenario: The file replaces the built-in list
- **WHEN** `server.toml` says `allow_origins = ["tilr://my-notes", "http://app.test"]`
- **THEN** `Origin: tilr://my-notes` connects and `Origin: tilr://app` gets 403

#### Scenario: The command line replaces the file
- **WHEN** the same file is there and the server runs with `--allow-origin http://cli.test`
- **THEN** `Origin: http://cli.test` connects and `Origin: tilr://my-notes` gets 403

#### Scenario: Misspelt key
- **WHEN** `server.toml` holds `allow_origin = []`
- **THEN** `tilrd` exits non-zero and its stderr names `server.toml` and `allow_origin`

### Requirement: The Unix socket is the user's alone and asks no token
The Unix socket (see `daemon-lifecycle` for its path) SHALL be created 0600, its lock file `<socket>.lock` 0600, and the runtime directory that holds it 0700; when the socket is in the default runtime directory the server MUST make that directory 0700 and MUST refuse to use one that belongs to another user. A `Hello` on the Unix socket MUST be accepted without a token, whatever `Hello.token` holds, also on a server that asks TCP clients for one.

**Verified by**: `local::the_socket_is_the_users_alone_and_asks_no_token`.

#### Scenario: Modes
- **WHEN** a server is serving its Unix socket
- **THEN** the socket's directory is mode 0700, the socket 0600 and its lock 0600

#### Scenario: Hello without a token on the socket
- **WHEN** a local client sends `Hello` with no token on the socket of a server that made a token
- **THEN** it receives `HelloAck` and can create and list panes

### Requirement: The Unix socket checks the peer's uid
For every connection accepted on the Unix socket the server SHALL read the peer's credentials (`SO_PEERCRED` on Linux, `getpeereid` on macOS) and MUST close at once, before any handshake, a connection whose uid is not the server's own, logging "refused a connection on the Unix socket". This MUST hold even where the directory and socket permissions were opened by hand.

**Verified by**: `local::another_user_is_turned_away_even_where_the_permissions_let_it_in` and itests UC-034 (`uc_034.py::another_user_cannot_use_the_socket`).

#### Scenario: Permissions keep another user out
- **WHEN** another uid runs `tilrd ps` against the socket with the default modes
- **THEN** it fails

#### Scenario: Permissions opened by hand
- **WHEN** the directories are made 0755 and the socket 0666, and another uid connects
- **THEN** the connection is refused, the server logs "refused a connection on the Unix socket", and the user's own `tilrd ps` still succeeds

### Requirement: The host-events and agent sockets check the peer's uid
The host-events socket (`$TILR_SOCK` in each pane, see `clipboard-host-events`) and each pane's SSH agent socket (`agent/<pid>/<pane id>.sock`, see `ssh-agent-forwarding`) SHALL be created 0600 in a directory of mode 0700 that belongs to the user, and the server MUST refuse every connection on them whose peer uid is not its own, logging "host socket: refused a connection" or "agent socket: refused a connection", even where the permissions were opened.

**Verified by**: `agent::a_pane_has_ssh_auth_sock_pointing_at_its_own_socket` (modes) and itests UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`).

#### Scenario: Another user sends tilr edit
- **WHEN** another uid connects to a pane's `$TILR_SOCK`, with the permissions opened, and sends a request
- **THEN** the connection is refused and no client receives a `HostEvent`

#### Scenario: Agent socket modes
- **WHEN** a pane is created with agent forwarding on
- **THEN** its `SSH_AUTH_SOCK` is mode 0600 in a directory of mode 0700

### Requirement: A handshake has 10 seconds
A connection SHALL be closed by the server when it has not completed its WebSocket handshake (or, on TCP, sent its first byte, finished its raw SOCKS5 negotiation or its `/metrics` request) within 10 s of being accepted, and a main-protocol connection that has not sent `Hello` within 10 s of its handshake with 4002 `timeout`, whatever `--client-timeout` is. The server MUST NOT otherwise limit the number of connections, and silent ones MUST NOT stop other clients.

**Verified by**: `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello`, itests UC-039 (`uc_039.py::a_silent_connection_is_closed_by_the_handshake_timeout`, `uc_039.py::a_websocket_that_never_says_hello_is_closed_after_10_s`) and UC-038 (`uc_038.py::a_thousand_silent_connections_and_a_slowloris_leave_the_server_working`).

#### Scenario: Connect and say nothing
- **WHEN** a client opens a TCP connection to the port and sends nothing
- **THEN** the server closes it about 10 s later

#### Scenario: WebSocket without Hello
- **WHEN** a client completes the WebSocket handshake with a server started with `--client-timeout 0`, answers its pings and sends no `Hello`
- **THEN** the server closes the connection with code 4002 `timeout` about 10 s after the handshake, and the log has `client sent no Hello in 10s`

#### Scenario: Slowloris
- **WHEN** a thousand connections sit silent and one sends its handshake a byte at a time
- **THEN** a normal client still connects and runs commands, and the silent ones are closed after 10 s

### Requirement: Cloud does not decide who connects
A client that comes through the Cloud tunnel SHALL be an ordinary TCP client of the server's own port: it MUST present the access token in `Hello` and pass the Origin check like any other. The server MUST NOT send the access token to Cloud (the link in `cloud.json` holds a separate machine token) and MUST NOT act on anything from Cloud's control connection but tunnel commands (see `cloud-tunnels`).

**Verified by**: itests UC-07A (`uc_07a.py::nothing_but_tunnel_commands_from_cloud_is_acted_on`, `uc_07a.py::a_server_without_authentication_gets_no_tunnel`).

#### Scenario: Cloud sends something else
- **WHEN** Cloud's control connection sends a command other than `tunnel_open` / `tunnel_close`
- **THEN** the server does nothing with it and no pane is touched

### Requirement: A connection gone before its uid check is not refused
On the Unix socket, the host socket and the agent sockets, a connection whose peer has closed before the server could read its credentials (a probe of whether a server is there; macOS cannot read them then) SHALL be closed without the `refused a connection` warning. A connection from another uid MUST still be refused and logged.

**Verified by**: `files::a_second_server_starting_leaves_the_first_ones_panes_their_tools` (the host socket) and itests UC-035 (`uc_035.py::another_user_cannot_send_tilr_edit_to_the_pane`, another uid still refused).

#### Scenario: A probe of the host socket
- **WHEN** a second server of the same user connects to the first one's `<server_id>.sock` and closes at once
- **THEN** the first server logs no `refused a connection` line

## Notes

- Requirements without a test: none. Partly untested: the constant-time comparison of tokens; "a tunnelled client needs the token" (follows from the tunnel leading to the TCP port, not tested end to end). "A connection gone before its uid check is not refused" is tested on the host socket only (the Unix and agent sockets share `local::peer_gone`; the warning appears on macOS only).
- `local::another_user_is_turned_away_even_where_the_permissions_let_it_in` returns early unless run as root, so on CI (`runner`) it checks nothing; itests UC-034 runs it as root.
- Unit tests in crates/tilrd/src/config.rs are cited by bare name (they are not in a `tests/` file).
- Code vs README: none found for these requirements.
- itests group D (UC-030..039) is all green at HEAD per itests/USE_CASES.md.
