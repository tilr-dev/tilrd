## MODIFIED Requirements

### Requirement: One client id per run
The client SHALL make one `client_id` (a UUID v4) when it starts and SHALL send the same `client_id` in the `Hello` of every connection it makes while it runs, reconnects included. What the server does with it is in the `client-liveness` spec.

**Verified by**: `client::reconnects_say_the_same_client_id`.

#### Scenario: Reconnect keeps the id
- **WHEN** an attached `tilr` loses its connection and connects again
- **THEN** the new `Hello.client_id` equals the one of the first connection

### Requirement: kill
`tilr kill [PANE]` SHALL close the named pane, else the pane it runs in (`$TILR_PANE`), else the pane used last, by sending `ClosePane` and polling `ListPanes` every 50 ms until the pane is gone. It SHALL fail with `pane <id> is still there` after 10 s and with `no panes` when there is none to choose.

**Verified by**: `client::kill_closes_the_named_pane_and_the_current_one`.

#### Scenario: Kill with no panes
- **WHEN** the server has no panes and the user runs `tilr kill`
- **THEN** it exits non-zero with `no panes`
