## ADDED Requirements

### Requirement: A client asks the server to listen for it
A client that lists `"forward"` in `Hello.features` SHALL be able to send `ForwardListen { request_id, port }` to a server offering `"forward"`; the server SHALL listen on `127.0.0.1:<port>` (a free port for 0) and answer `ForwardListening { request_id, forward_id, port }`, or with `error` set (`address in use`, `too many forwards` beyond 16 per client, `forwarding is turned off on this server`).

**Verified by**: `forward::a_client_asks_the_server_to_listen_for_it` (crates/tilrd/tests/forward.rs).

#### Scenario: A free port
- **WHEN** a client sends `ForwardListen` with port 0
- **THEN** it receives `ForwardListening` with a non-zero `forward_id` and `port`, and `127.0.0.1:<port>` accepts TCP connections on the server's machine

#### Scenario: Turned off
- **WHEN** the server runs with forwarding off and a client sends `ForwardListen`
- **THEN** `HelloAck.features` lacks `"forward"` and the answer's `error` is `forwarding is turned off on this server`

### Requirement: A connection to a forward opens a stream to its client
Each TCP connection to a forward's port SHALL open a stream to the client that asked for it, `StreamOpen { kind: "tcp-forward", forward_id }`, and its bytes SHALL go both ways unchanged and in order as `StreamData`, ending with `StreamClose` from either side, under the same per-client stream limit, bulk-lane backpressure and queue rules as agent streams.

**Verified by**: `forward::bytes_go_both_ways_through_a_forward` and itests UC-07E (`uc_07e.py::psql_on_the_server_reaches_postgres_on_the_laptop_through_tilr_forward`).

#### Scenario: Echo through a forward
- **WHEN** a program on the server's machine connects to the forward's port and sends "ping", and the client echoes the stream
- **THEN** the program reads "ping", and when the client sends `StreamClose` the program reads end of file

#### Scenario: A slow client
- **WHEN** a program sends 64 MiB through a forward to a client reading 1 MB/s
- **THEN** the server reads from the program only as the client's bulk lane has room and its memory stays bounded

### Requirement: A forward ends with its client
A forward SHALL end, its port closed, on `ForwardStop { forward_id }` from its client and when its client's connection ends for any reason, a reload included; open streams of a stopped forward SHALL go on until closed, and those of a gone client SHALL end.

**Verified by**: `forward::a_forward_ends_with_its_client` and `reload::forwards_end_with_a_reload_and_tilr_asks_again`.

#### Scenario: The client disconnects
- **WHEN** the client that asked for a forward disconnects
- **THEN** connecting to the forward's port is refused

#### Scenario: Reload
- **WHEN** the server reloads while `tilr forward -R` runs
- **THEN** after `tilr` reconnects the same port forwards again

### Requirement: tilr forward
`tilr forward -R PORT:HOST:HPORT` SHALL ask the server for the forward and connect each stream to `HOST:HPORT` on the user's machine; `tilr forward -L [ADDR:]LPORT:HOST:HPORT` SHALL listen on `ADDR:LPORT` (default `127.0.0.1`) and carry each connection to `HOST:HPORT` through the server's SOCKS proxy, failing with `the server's SOCKS proxy is off` when it is. Both SHALL run until interrupted and ask again for their forwards after reconnecting.

**Verified by**: `client::tilr_forward_carries_connections_both_ways` (crates/tilr/tests/client.rs).

#### Scenario: Reverse
- **WHEN** a service listens on the user's `127.0.0.1:5432` and `tilr forward -R 15432:127.0.0.1:5432` runs
- **THEN** a program on the server's machine connecting to `127.0.0.1:15432` talks to that service

#### Scenario: Local without SOCKS
- **WHEN** the server runs without `--socks` and `tilr forward -L 8080:127.0.0.1:80` runs
- **THEN** it exits non-zero with `the server's SOCKS proxy is off`
