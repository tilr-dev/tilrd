## MODIFIED Requirements

### Requirement: TCP clients present the token in Hello
On the TCP port, a server that asks for a token SHALL accept a `Hello` only when `Hello.token` equals its token, compared in time that does not depend on where they differ. A `Hello` without a token MUST be answered `Error { message: "this server asks for its access token" }`, one with another token `Error { message: "wrong access token" }`, and the connection then closed, with nothing the client sent acted on. This holds whatever address the server listens on, loopback or not.

This is the `tilr.v1` protocol. On the bus (`tilr.bus.v1`, see `bus-acl`) a server that asks for a token SHALL accept a `Hello` on the TCP port that carries the access token (`Hello.access_token`, compared the same way) or a valid `credential` of this server (a token minted by `bus.token`, which then stands in for the access token: the connection is that token's participant, with its rights only). A bus `Hello` with neither MUST be answered `ConnectionError { code: "denied", message: "this server asks for its access token or a credential" }`, one with another access token and no credential `ConnectionError { code: "denied", message: "wrong access token" }`, and one with a `credential` that does not verify `ConnectionError { code: "denied" }` naming only that it is not a token of this server (or that it is expired, revoked or bound to another origin), also when it carries the right access token; the connection is then closed with nothing the client sent acted on.

**Verified by**: `auth::a_client_needs_the_access_token` and itests UC-031 (`uc_031.py::listening_on_every_ipv4_address_asks_for_the_token`, `uc_031.py::listening_on_every_address_takes_ipv4_too_and_asks_for_the_token`); on the bus `bus_acl::a_tcp_participant_needs_a_token` and `supervisor::bus::acl::tests::hello_with_a_credential_is_its_participant`.

#### Scenario: No token
- **WHEN** a TCP client sends `Hello` with an empty `token`
- **THEN** it receives `Error` "this server asks for its access token" and the connection closes

#### Scenario: Wrong token
- **WHEN** a TCP client sends `Hello` with a token that differs in one character
- **THEN** it receives `Error` "wrong access token" and the connection closes

#### Scenario: Right token
- **WHEN** a TCP client sends `Hello` with the token
- **THEN** it receives `HelloAck`

#### Scenario: Bus Hello with a credential only
- **WHEN** a bus client on the TCP port says `Hello` with a token the owner minted with `bus.token` and no access token
- **THEN** it receives `HelloAck` with the identity `token:<name>`

#### Scenario: Bus Hello with neither
- **WHEN** a bus client on the TCP port says `Hello` with neither `access_token` nor `credential`
- **THEN** it receives `ConnectionError` `denied` "this server asks for its access token or a credential" and the connection closes

#### Scenario: Bus Hello with a bad credential
- **WHEN** a bus client on the TCP port says `Hello` with a `credential` that is not a token of this server, with or without the right access token
- **THEN** it receives `ConnectionError` `denied` "not a token of this server" and the connection closes

### Requirement: Cloud does not decide who connects
A client that comes through the Cloud tunnel SHALL be an ordinary TCP client of the server's own port: it MUST present the access token in `Hello` and pass the Origin check like any other. The server MUST NOT send the access token to Cloud (the link in `cloud.json` holds a separate machine token) and MUST NOT act on anything from Cloud's control connection but tunnel commands (see `cloud-tunnels`).

On the bus (`tilr.bus.v1`) such a client SHALL, like any TCP client of the port (requirement "TCP clients present the token in Hello"), present the access token or a valid `credential` of this server in `Hello`, and with neither MUST be refused; the tunnel's connection is a connection to the port, so the same checks apply to it.

**Verified by**: itests UC-07A (`uc_07a.py::nothing_but_tunnel_commands_from_cloud_is_acted_on`, `uc_07a.py::a_server_without_authentication_gets_no_tunnel`); on the bus `bus_acl::a_tcp_participant_needs_a_token` (the port's checks, which a tunnel's connection reaches as any TCP connection).

#### Scenario: Cloud sends something else
- **WHEN** Cloud's control connection sends a command other than `tunnel_open` / `tunnel_close`
- **THEN** the server does nothing with it and no pane is touched

#### Scenario: A bus client through the tunnel
- **WHEN** a bus client reaches the port through the tunnel and says `Hello` with a minted token and no access token, and another with neither
- **THEN** the first is that token's participant and the second receives `ConnectionError` `denied`
