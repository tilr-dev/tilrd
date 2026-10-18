## ADDED Requirements

### Requirement: A superseded attach ends
When the server closes an attached `tilr`'s connection with close code 4000 (`superseded`), `tilr` SHALL NOT reconnect: it SHALL restore the terminal, print `[superseded by another connection of this client]` on stdout and exit with status 0.

**Verified by**: `client::an_attach_superseded_by_its_own_client_id_ends` (planned in this change).

#### Scenario: Another connection with the same client id
- **WHEN** `tilr` is attached and a second connection says `Hello` with `tilr`'s `client_id` and the same token on the same transport
- **THEN** `tilr` prints `[superseded by another connection of this client]` and exits 0 without connecting again
- **AND** the pane goes on
