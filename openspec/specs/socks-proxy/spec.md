# socks-proxy Specification

## Purpose

Covers the SOCKS5 proxy (RFC 1928, CONNECT only) the server serves on its own TCP port when asked to, so that a program on another machine reaches services on the server's machine (its loopback included): turning it on, the `/socks` WebSocket and raw SOCKS5 ways in, their authentication, name resolution, replies, limits and the session lifetime. The access token and allowed origins as such are owned by the `access-control` spec; the TCP listener and the 10 s handshake timeout of the port by the `daemon-lifecycle` spec.

## Requirements

### Requirement: The proxy is off unless asked for
The server SHALL serve the proxy only with `--socks` or `socks = true` in its configuration file (unset means off), logging `SOCKS proxy on: WebSocket /socks and raw SOCKS5 on this port` or `SOCKS proxy off` at start. While off, a `/socks` handshake MUST be refused with HTTP 404 and a raw SOCKS5 connection closed without an answer; the main protocol is unchanged.

**Verified by**: `socks::the_proxy_is_off_unless_asked_for` and `socks::the_configuration_file_turns_the_proxy_on`.

#### Scenario: Default server
- **WHEN** a client opens `/socks` with a valid `Authorization: Bearer <token>` on a server started without `--socks`
- **THEN** the handshake fails with 404
- **AND** a raw client sending `05 01 02` reads end of file or an error, and the main protocol still lists panes

#### Scenario: Configuration file
- **WHEN** `server.toml` holds `socks = true`
- **THEN** the log says `SOCKS proxy on` and a `/socks` WebSocket with the token is accepted

### Requirement: The SOCKS WebSocket is on the TCP port only
`GET /socks` SHALL be served only on the TCP port; on the Unix socket a handshake for `/socks` MUST be refused with 404 whether the proxy is on or off. Any other path is the main protocol.

**Verified by**: `socks::the_socks_path_is_not_served_on_the_unix_socket`.

#### Scenario: Over the Unix socket
- **WHEN** a client opens a WebSocket for `/socks` on the server's Unix socket of a server run with `--socks`
- **THEN** the handshake is refused with 404

### Requirement: The SOCKS WebSocket handshake shows the access token
The `/socks` handshake SHALL carry the access token as `Authorization: Bearer <token>` or, for a page, which cannot set WebSocket headers, `?token=<token>`. Without it or with a wrong one the handshake MUST be refused with 401 and `WWW-Authenticate: Bearer`. `--no-auth`: none.

**Verified by**: `socks::the_socks_websocket_needs_the_access_token`, `socks::a_page_reaches_a_loopback_service_through_the_socks_websocket` and `socks::a_page_that_cannot_set_headers_gives_the_token_in_the_query`.

#### Scenario: Wrong or missing token
- **WHEN** `/socks` is opened with no token, with `Authorization: Bearer nope` or with `?token=nope`
- **THEN** each handshake fails with 401

#### Scenario: Token in the query
- **WHEN** `/socks?token=<token>` is opened without headers
- **THEN** the WebSocket opens and a CONNECT through it succeeds

#### Scenario: Without authentication
- **WHEN** the server runs with `--socks --no-auth` and `/socks` is opened without a token
- **THEN** the WebSocket opens and a CONNECT through it succeeds

### Requirement: The SOCKS WebSocket comes only from an allowed origin
A `/socks` handshake that carries an `Origin` not allowed for the main protocol SHALL be refused with 403, before the token is looked at.

**Verified by**: `socks::the_socks_websocket_comes_only_from_an_allowed_origin`.

#### Scenario: Another site's page
- **WHEN** the server runs with `--allow-origin http://app.test` and `/socks` is opened with the token and `Origin: http://evil.test`
- **THEN** the handshake fails with 403
- **AND** the same request with `Origin: http://app.test` succeeds

### Requirement: Binary messages carry the SOCKS bytes
Over `/socks`, the client's binary messages SHALL be the SOCKS bytes it sends and the server's binary messages the bytes it gets back, in any sizes. SOCKS asks for no authentication there: the server picks method `00` when offered, else `02`, whose username and password are taken unread.

**Verified by**: `socks::a_page_reaches_a_loopback_service_through_the_socks_websocket`.

#### Scenario: A large exchange
- **WHEN** 300 000 bytes are sent to an echo service through `/socks`
- **THEN** they come back whole, in more than one message

### Requirement: Close codes of the SOCKS WebSocket
A text message from the client SHALL close the `/socks` WebSocket with code 1003. Since a WebSocket cannot be half closed, the server MUST close it with 1000 once the target is done sending or the session has ended (as after a refused CONNECT).

**Verified by**: `socks::a_text_message_closes_the_socks_websocket` and `socks::only_connect_is_served_and_a_refusal_is_reported`.

#### Scenario: A text message
- **WHEN** a client sends the text message "hello" on an open `/socks` WebSocket
- **THEN** the server closes it with 1003

### Requirement: A silent SOCKS WebSocket is dropped
The server SHALL ping a `/socks` WebSocket three times within the client timeout (`--client-timeout`), as for the main protocol, and MUST drop one that sends nothing for that long; the target connection then ends.

**Verified by**: itests `UC-047` (`uc_047.py::a_socks_stream_on_a_silent_connection_ends_for_the_service_and_the_next_one_works`).

#### Scenario: The network goes silent
- **WHEN** the connection of an open SOCKS session is frozen
- **THEN** within the client timeout plus 3 s the service on the server's machine sees its connection end
- **AND** a new session to the same service works

### Requirement: Raw SOCKS5 on the same port gives the token as the password
A TCP connection to the server's port whose first byte is 5 SHALL be a raw SOCKS5 session, wherever `--listen` listens. It MUST authenticate by username/password (RFC 1929), the password being the access token, any username: no method `02` offered gets `05 FF`, a wrong password status `01`, each then closed. A `--no-auth` server asks for none (method `00`). The main protocol shares the port.

**Verified by**: `socks::a_raw_socks_client_gives_the_token_as_its_password`; `socks::a_raw_socks_client_of_a_server_without_authentication_needs_no_password`.

#### Scenario: Token as password
- **WHEN** a raw client offers methods `00 02` and logs in as "anyone" with the token
- **THEN** the server chooses `02`, answers status `00`, and a CONNECT to an echo service succeeds

#### Scenario: Wrong password or none
- **WHEN** a raw client logs in with the password "wrong", or offers only method `00`
- **THEN** it gets status `01`, or method `FF`, and the connection is closed

#### Scenario: Without authentication
- **WHEN** the server runs with `--socks --no-auth` and a raw client offers method `00`
- **THEN** the server chooses `00` and a CONNECT succeeds

### Requirement: CONNECT only
The proxy SHALL serve the CONNECT command only: BIND and UDP ASSOCIATE MUST be answered with reply `07` (command not supported), an address type other than IPv4, domain name or IPv6 with reply `08` (address type not supported), and the session then ends. A CONNECT that succeeds MUST be answered `00` with BND.ADDR and BND.PORT the address the server connected from.

**Verified by**: `socks::only_connect_is_served_and_a_refusal_is_reported`.

#### Scenario: BIND
- **WHEN** a client asks for BIND
- **THEN** it gets reply `07`

### Requirement: A failed connection is reported with its reason
When the connection to the target fails, the server SHALL answer with reply `05` (connection refused), `03` (network unreachable), `04` (host unreachable, also for a name that does not resolve and for a connect taking longer than 10 s) or `01` (any other failure), and end the session; over `/socks` the WebSocket is then closed with 1000. For a name with several addresses, each is tried in turn and the last failure reported.

**Verified by**: `socks::only_connect_is_served_and_a_refusal_is_reported`.

#### Scenario: A closed port
- **WHEN** a client asks to CONNECT to a port on which nothing listens
- **THEN** it gets reply `05` and the WebSocket is closed with 1000

### Requirement: Names are resolved on the server and localhost is its loopback
A domain name in a CONNECT SHALL be resolved on the server ("remote DNS"). `localhost` and every name under it (any case, a trailing dot allowed; RFC 6761) MUST NOT go to DNS: they reach the server's loopback, 127.0.0.1 first, then `[::1]`.

**Verified by**: `socks::any_name_under_localhost_is_the_servers_loopback`, `socks::a_page_that_cannot_set_headers_gives_the_token_in_the_query` and itests `UC-074` (`uc_074.py::curl_on_the_clients_machine_reaches_a_service_on_the_servers_loopback`).

#### Scenario: A name that exists nowhere
- **WHEN** a client asks to CONNECT to `nonexistent.LocalHost` on the port of a service bound to the server's 127.0.0.1
- **THEN** it gets reply `00` and talks to that service

#### Scenario: curl on another machine
- **WHEN** curl on the client's machine uses `--socks5-hostname` through the client's local SOCKS port, which carries each connection to the server's `/socks`, to fetch from a service bound only to the server's 127.0.0.1
- **THEN** it gets the service's page

### Requirement: The client must say where it goes within 10 s
A SOCKS session SHALL be given 10 s from its first byte to finish the negotiation and the request; one that has not MUST be closed. (The time to the first byte is the port's handshake timeout, see the `daemon-lifecycle` spec.)

**Verified by**: `socks::a_session_that_does_not_say_where_it_goes_is_closed`.

#### Scenario: A client that stalls
- **WHEN** a raw SOCKS client sends its greeting and then nothing for 10 s
- **THEN** the server closes the connection

### Requirement: Sessions run side by side and with the terminal
Every SOCKS session SHALL run on its own: many at once each get their own data, a large response arrives whole, a session that has not yet said where it goes holds no other up, and the main protocol on the same port keeps working.

**Verified by**: `socks::socks_sessions_run_side_by_side_with_the_terminal` and itests `UC-074` (`uc_074.py::twenty_sessions_at_once_each_get_their_own_file`, `uc_074.py::a_big_response_through_socks_arrives_whole`).

#### Scenario: Twenty at once
- **WHEN** twenty curl downloads of different 1 MiB files run at once through the proxy
- **THEN** each gets its own file, byte for byte

#### Scenario: An idle session
- **WHEN** one `/socks` WebSocket has sent only the byte 5 while two others are connected to an echo service
- **THEN** the other two still echo and a main-protocol client lists panes

### Requirement: A session ends with its connection and with the server
A SOCKS session SHALL end, and the target see its connection end, when the client's connection is reset or closed, and when the server reloads or updates; the next session works.

**Verified by**: itests `UC-047` (`uc_047.py::a_socks_stream_cut_by_a_reset_ends_for_the_service_and_the_next_one_works`) and `UC-064` (`uc_064.py::a_socks_session_open_during_an_update_ends_for_the_service_and_a_new_one_works`).

#### Scenario: A reset mid-session
- **WHEN** the connection of an open SOCKS session is reset
- **THEN** within 3 s the service on the server's machine sees its connection end
- **AND** a new session echoes 200 000 bytes

#### Scenario: An update
- **WHEN** the server updates while a SOCKS session is open
- **THEN** the service sees that session end and a new session works

## Notes

- Parts without a test: replies `03`, `04`, `08` and `01`; the 10 s connect timeout.
- No itests case cited here is currently red per itests/USE_CASES.md.
- The proxy does not use the protocol's `StreamOpen` / `StreamData` / `StreamClose`: each SOCKS session is a WebSocket (`/socks`) or TCP connection of its own. "Through the client" (UC-074) means the client on the user's machine opening a local SOCKS port and carrying each connection to `/socks` (tilr-driver's `socks_listen`); the `tilr` client at HEAD has no such command.
- UC-085 (foreign Origin / DNS rebinding on `/socks`) is listed in itests/USE_CASES.md without a scenario.
