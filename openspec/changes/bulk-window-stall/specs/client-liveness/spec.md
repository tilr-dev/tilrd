## ADDED Requirements

### Requirement: Any pong reopens a closed bulk window
While a client's bulk window is closed, every ping the server sends SHALL carry the current bulk position, so that a pong to any of them MUST let the window reopen.

#### Scenario: Client answers only the newest ping
- **WHEN** a client reading a large file answers only the most recent of several pings
- **THEN** the whole file arrives

### Requirement: A lagging client gets its last screen
When output stops while a client lags, the server SHALL still send that client the latest full frame of each pane it watches, without waiting for more output.

#### Scenario: Output stops during lag
- **WHEN** a client lags while a pane prints and the pane then goes quiet
- **THEN** the client receives a full frame matching the pane's final screen

### Requirement: A pong cannot widen the window
The server SHALL take a pong's position as at most the number of bulk bytes it has sent to that client.

#### Scenario: Pong ahead of the data
- **WHEN** a client answers with a position beyond what was sent
- **THEN** the unread amount is computed from the bytes sent and stays within 1 MiB
