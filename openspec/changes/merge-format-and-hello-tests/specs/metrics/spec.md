## MODIFIED Requirements

### Requirement: Clients let go, by reason
`tilrd_client_drops_total{reason}` SHALL count the clients the server let go, with a sample from 0 for each of `superseded` (by a newer connection of the same client), `too_slow` (its queue filled up), `silent` (nothing heard for the client timeout), `no_pong` (it sent, but answered no ping for the client timeout) and `no_hello` (no `Hello` within 10 s of its WebSocket handshake). A client that disconnects on its own, or that a handshake check turns away with an `Error`, SHALL NOT be counted.

**Verified by**: `metrics::metrics_count_the_clients_let_go_and_why`, `auth::a_connection_that_never_says_hello_is_closed_with_4002_and_counted_as_no_hello` and itests `UC-052` (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`).

#### Scenario: Superseded
- **WHEN** a second connection says hello with the `client_id` and token of an open one
- **THEN** `tilrd_client_drops_total{reason="superseded"}` goes from 0 to 1 and `too_slow` stays 0

#### Scenario: Superseded over the network
- **WHEN** a client's connection is superseded (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`)
- **THEN** `tilrd_client_drops_total{reason="superseded"}` is 1

#### Scenario: No Hello
- **WHEN** a client completes the WebSocket handshake, sends no `Hello`, and is closed with 4002 `timeout` about 10 s later
- **THEN** `tilrd_client_drops_total{reason="no_hello"}` goes from 0 to 1 and the other reasons stay 0

#### Scenario: A wrong token is not a drop
- **WHEN** a TCP client says `Hello` with a wrong token and is answered with an `Error`
- **THEN** no `tilrd_client_drops_total` sample changes
