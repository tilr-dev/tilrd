## MODIFIED Requirements

### Requirement: A new connection starts afresh
A new connection SHALL subscribe again and receive a full first pane frame. Clients with stable identities SHALL retain admitted numbered input, reconcile applied counters from `HelloAck`, and replay the remainder in order, exactly once. A connection closed with 4000 `superseded` MUST NOT reconnect.

**Verified by**: `reconnect::a_client_that_reconnects_finds_its_pane_as_it_left_it`, `reconnect::subscribing_again_sends_the_whole_screen_again`, `input_delivery::disconnect_before_application_replays_one_copy`, `input_delivery::applied_input_with_a_lost_ack_is_not_applied_twice` and itests UC-048 (`uc_048.py`) and UC-052 (`uc_052.py::a_superseded_connection_is_closed_with_reason_superseded_and_not_reconnected`); for `tilr`, `client::an_attach_superseded_by_its_own_client_id_ends`.

#### Scenario: Reconnect
- **WHEN** a client reconnects and subscribes to the pane it watched
- **THEN** its first frame is full and the shell is the same process as before

#### Scenario: Input at the moment of a cut
- **WHEN** a burst of input lines is cut by a reset and the client reconnects
- **THEN** each line admitted to its delivery queue reaches the pane exactly once, in order

#### Scenario: An old connection revives
- **WHEN** a client replays unacknowledged input on a new connection and its old connection revives
- **THEN** accepted delivery sequences are deduplicated and every admitted input is applied exactly once
