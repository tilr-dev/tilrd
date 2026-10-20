## MODIFIED Requirements
### Requirement: Watching a pane
`Subscribe` SHALL add the connection to the pane's viewers and be followed by a full frame (see `screen-frames`); a viewer SHALL get the pane's frames until it unsubscribes, the pane exits or the connection ends. `PaneInfo.viewers` SHALL count the subscribed connections and `viewed_by` list one `Viewer` per connection (see `client-liveness`). The first frame delivered after each `Subscribe` SHALL be full.

**Verified by**: `reconnect::subscribing_again_sends_the_whole_screen_again`, `reconnect::a_client_back_on_a_new_connection_supersedes_its_old_one_at_once` and `supervisor::tests::a_subscriber_gets_full_before_delta_and_keeps_its_own_input_echo`.

#### Scenario: Subscribe while delta is in flight
- **GIVEN** a pane with an existing viewer and an in-flight delta
- **WHEN** another connection subscribes before that delta reaches the supervisor
- **THEN** the existing viewer receives the delta, the new viewer receives no delta before a full frame, and both receive subsequent frames normally with existing backpressure

#### Scenario: Own input echoed before the initial full frame
- **WHEN** delta skipped after Subscribe acknowledges the subscriber's own input, followed by a full frame acknowledging another client's input
- **THEN** the subscriber receives the full frame with its latest skipped own input sequence

#### Scenario: Subscribe again while recovering
- **WHEN** a connection subscribes again before receiving the full frame that ends its recovery
- **THEN** its recorded own input acknowledgement is preserved until the full frame arrives

#### Scenario: Subscribing again
- **WHEN** a client that watches a pane subscribes to it again
- **THEN** it gets a full frame of the pane
